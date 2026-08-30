import { Alerter } from '../alert/alerter';

export interface RandomnessJob {
  requestId: bigint;
  raffleContract: string;
  timestamp: bigint;
}

export interface RequestQueueOptions {
  alerter?: Alerter;
  depthLimit?: number;
  ageLimitMs?: number;
}

export class RequestQueue {
  private readonly jobs: RandomnessJob[] = [];
  private readonly enqueuedAtMs: number[] = [];
  private readonly alerter?: Alerter;
  private readonly depthLimit: number;
  private readonly ageLimitMs: number;

  constructor(options: RequestQueueOptions = {}) {
    this.alerter = options.alerter;
    this.depthLimit = options.depthLimit ?? Number(process.env.ALERT_QUEUE_DEPTH_LIMIT ?? 10);
    this.ageLimitMs = options.ageLimitMs ?? Number(process.env.ALERT_QUEUE_AGE_LIMIT_MS ?? 300_000);
  }

  enqueue(job: RandomnessJob): void {
    this.jobs.push(job);
    this.enqueuedAtMs.push(Date.now());
  }

  drain(): RandomnessJob[] {
    const pending = [...this.jobs];
    this.jobs.length = 0;
    this.enqueuedAtMs.length = 0;
    return pending;
  }

  size(): number {
    return this.jobs.length;
  }

  /**
   * Alerts when the queue has grown too deep or the oldest request has been
   * waiting too long. Designed to be called on a fixed schedule by the worker.
   */
  checkHealth(now: number = Date.now()): void {
    if (!this.alerter) {
      return;
    }

    const depth = this.size();
    if (depth > this.depthLimit) {
      void this.alerter.notify({
        type: 'queue_depth',
        severity: 'warning',
        message: `Request queue depth (${depth}) exceeds limit (${this.depthLimit})`,
        details: { depth, limit: this.depthLimit },
      });
    }

    const oldestEnqueuedAt = this.enqueuedAtMs[0];
    if (oldestEnqueuedAt !== undefined && now - oldestEnqueuedAt > this.ageLimitMs) {
      const ageMs = now - oldestEnqueuedAt;
      void this.alerter.notify({
        type: 'queue_age',
        severity: 'warning',
        message: `Oldest queued request is ${ageMs}ms old (limit ${this.ageLimitMs}ms)`,
        details: { ageMs, limit: this.ageLimitMs },
      });
    }
  }
}
