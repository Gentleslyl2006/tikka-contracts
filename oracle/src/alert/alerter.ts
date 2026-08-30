export class Alerter {
  private consecutiveFailures = 0;

  constructor(private readonly threshold: number = 3) {}

  recordSuccess(): void {
    this.consecutiveFailures = 0;
  }

  recordFailure(): void {
    this.consecutiveFailures++;
    if (this.consecutiveFailures >= this.threshold) {
      this.fire();
    }
  }

  private fire(): void {
    console.error(
      `ALERT: ${this.consecutiveFailures} consecutive tx-submission failures`,
    );
  }
}
