import { EventListenerService } from './listener/event-listener.service';
import { RequestQueue } from './queue/request-queue';
import { FileLedgerCheckpointStore, LedgerCheckpointStore } from './listener/ledger-checkpoint';
import { KeyService } from './keys/key.service';
import { VrfService } from './vrf/vrf.service';
import { TxSubmitterService } from './tx/tx-submitter.service';
import { DeduplicationStore } from './deduplication/deduplication.store';
import { GracefulShutdown } from './shutdown/graceful-shutdown';
import { Alerter } from './alert/alerter';
import { OracleConfig } from './config';

export interface PipelineOptions {
  config: OracleConfig;
  alerter: Alerter;
  checkpointStore?: LedgerCheckpointStore;
  dedupStore?: DeduplicationStore;
}

export class OraclePipeline {
  private readonly keyService: KeyService;
  private eventListener: EventListenerService;
  private readonly requestQueue: RequestQueue;
  private readonly vrfService: VrfService;
  private readonly txSubmitter: TxSubmitterService;
  private readonly dedupStore: DeduplicationStore;
  private readonly checkpointStore: LedgerCheckpointStore;
  private readonly gracefulShutdown: GracefulShutdown;
  private readonly alerter: Alerter;
  private readonly config: OracleConfig;

  constructor(options: PipelineOptions) {
    const { config, alerter, checkpointStore, dedupStore } = options;

    this.config = config;
    this.alerter = alerter;

    // Initialize KeyService (must be called before accessing public key)
    this.keyService = new KeyService();
    // Note: initialize() is called in start() to allow async constructor pattern

    // Initialize checkpoint store
    this.checkpointStore = checkpointStore ?? new FileLedgerCheckpointStore('./data/checkpoint.json');

    // Initialize deduplication store
    this.dedupStore = dedupStore ?? new DeduplicationStore('./data/dedup.json');

    // Initialize request queue
    this.requestQueue = new RequestQueue();

    // Initialize VRF service
    this.vrfService = new VrfService(this.keyService);

    // Initialize transaction submitter
    this.txSubmitter = new TxSubmitterService(this.keyService, {
      rpcUrl: config.rpcUrl,
      alerter: this.alerter,
      failureThreshold: config.alertFailureThreshold,
    });

    // Initialize event listener (public key will be available after initialize)
    this.eventListener = new EventListenerService(
      this.requestQueue,
      '', // Placeholder; will be set after initialization
      this.checkpointStore,
      {
        rpcUrl: config.rpcUrl,
        pollIntervalMs: config.pollIntervalMs,
        alerter: this.alerter,
        rpcUnreachableThreshold: config.alertRpcUnreachableThreshold,
      }
    );

    // Initialize graceful shutdown
    this.gracefulShutdown = new GracefulShutdown(
      this.requestQueue,
      this.checkpointStore,
      {
        drainTimeoutMs: 30_000, // 30 seconds
        processJob: this.processJob.bind(this),
        exitFn: (code) => {
          void alerter
            .notify({
              type: 'process_stop',
              severity: code === 0 ? 'info' : 'critical',
              message: `Oracle service ${code === 0 ? 'stopped' : 'failed'} (exit code ${code})`,
            })
            .finally(() => process.exit(code));
        },
      }
    );
  }

  async start(contractIds: string[]): Promise<void> {
    console.log(`Starting oracle service for contracts: ${contractIds.join(', ')}`);

    // Initialize KeyService
    await this.keyService.initialize();

    // Create event listener with actual public key
    this.eventListener = new EventListenerService(
      this.requestQueue,
      this.keyService.getPublicKey(),
      this.checkpointStore,
      {
        rpcUrl: this.config.rpcUrl,
        pollIntervalMs: this.config.pollIntervalMs,
        alerter: this.alerter,
        rpcUnreachableThreshold: this.config.alertRpcUnreachableThreshold,
      }
    );

    // Initialize event listener (loads checkpoint or starts from current ledger)
    await this.eventListener.initialize();

    // Register graceful shutdown handlers
    this.gracefulShutdown.register(() => this.eventListener.stopListening());

    process.on('SIGINT', () => this.gracefulShutdown.shutdown());
    process.on('SIGTERM', () => this.gracefulShutdown.shutdown());

    // Start processing jobs from the queue
    this.processQueue();

    // Start listening for events
    await this.eventListener.startListening(contractIds);

    console.log('Oracle service started successfully');
  }

  private async processJob(job: { requestId: bigint; raffleContract: string; timestamp: bigint }): Promise<boolean> {
    const { requestId, raffleContract } = job;

    // Check for duplicates
    if (this.dedupStore.isDuplicate(requestId, raffleContract)) {
      console.log(`Skipping duplicate request: raffle=${raffleContract} requestId=${requestId}`);
      return false;
    }

    try {
      // Generate proof and derive the seed from the proof itself.
      const proof = this.vrfService.signRandomnessProof(raffleContract, requestId);

      // Submit transaction
      const txHash = await this.txSubmitter.submitProvideRandomness({
        raffleContract,
        randomSeed: proof.randomSeed,
        publicKey: proof.publicKey,
        proof: proof.proof,
        requestId,
      });

      console.log(`Successfully submitted provide_randomness: ${txHash} for raffle=${raffleContract} requestId=${requestId}`);

      // Mark as processed (after successful submission)
      this.dedupStore.isDuplicate(requestId, raffleContract); // This marks it as seen

      return true;
    } catch (error) {
      console.error(`Failed to process job raffle=${raffleContract} requestId=${requestId}:`, error);
      throw error;
    }
  }

  private async processQueue(): Promise<void> {
    while (true) {
      const jobs = this.requestQueue.drain();
      if (jobs.length === 0) {
        await new Promise((resolve) => setTimeout(resolve, 100)); // Poll for new jobs
        continue;
      }

      for (const job of jobs) {
        try {
          await this.processJob(job);
        } catch (error) {
          console.error('Error processing job:', error);
          // Job will be retried on next restart if not marked as duplicate
        }
      }
    }
  }

  async shutdown(): Promise<void> {
    console.log('Shutting down oracle service...');
    await this.gracefulShutdown.shutdown();
  }
}
