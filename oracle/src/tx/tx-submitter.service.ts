import {
  Account,
  Contract,
  Networks,
  rpc as SorobanRpc,
  TransactionBuilder,
  nativeToScVal,
} from '@stellar/stellar-sdk';
import { KeyService } from '../keys/key.service';
import { RetryPolicy, RetryClass } from './retry-policy';
import { Alerter } from '../alert/alerter';

export interface ProvideRandomnessParams {
  raffleContract: string;
  randomSeed: bigint;
  publicKey: Uint8Array;
  proof: Uint8Array;
  requestId: bigint;
}

export interface SubmitResult {
  hash: string;
  attempts: number;
}

export class TxSubmitterService {
  private readonly server: SorobanRpc.Server;
  private sequenceCache?: string;
  private feeBumpCount = 0;
  private timeoutMultiplier = 1;

  constructor(
    private readonly keyService: KeyService,
    private readonly retryPolicy: RetryPolicy = new RetryPolicy(),
    private readonly alerter: Alerter = new Alerter(
      parseInt(process.env.ALERT_FAILURE_THRESHOLD ?? '3', 10),
    ),
    rpcUrl: string = process.env.STELLAR_RPC_URL ?? 'https://soroban-testnet.stellar.org',
    private readonly networkPassphrase: string = process.env.STELLAR_NETWORK_PASSPHRASE ??
      Networks.TESTNET,
  ) {
    this.server = new SorobanRpc.Server(rpcUrl, { allowHttp: rpcUrl.startsWith('http://') });
  }

  async submitProvideRandomness(params: ProvideRandomnessParams): Promise<SubmitResult> {
    let lastError: Error | undefined;

    for (let attempt = 0; attempt < this.retryPolicy.maxAttempts; attempt++) {
      try {
        const hash = await this.submitOnce(params);
        this.alerter.recordSuccess();
        return { hash, attempts: attempt + 1 };
      } catch (err) {
        lastError = err instanceof Error ? err : new Error(String(err));
        const decision = this.retryPolicy.classify(lastError);

        if (!decision.retry) {
          this.alerter.recordFailure();
          throw new Error(
            `Permanent failure submitting provide_randomness (${decision.class}): ${lastError.message}`,
          );
        }

        if (decision.action === 'refresh-sequence') {
          this.sequenceCache = undefined;
        }

        if (decision.action === 'bump-fee') {
          this.feeBumpCount++;
        }

        if (decision.action === 'rebuild-bounds') {
          this.timeoutMultiplier *= 2;
        }

        this.alerter.recordFailure();

        if (attempt < this.retryPolicy.maxAttempts - 1) {
          const delay = this.retryPolicy.nextDelay(attempt);
          await this.sleep(delay);
        }
      }
    }

    throw new Error(
      `Failed to submit provide_randomness after ${this.retryPolicy.maxAttempts} attempts: ${lastError?.message}`,
    );
  }

  private async submitOnce(params: ProvideRandomnessParams): Promise<string> {
    const keypair = this.keyService.getKeypair();
    const account = await this.server.getAccount(keypair.publicKey());
    const sequence = this.sequenceCache ?? account.sequenceNumber();
    const sourceAccount = new Account(account.accountId(), sequence);

    const contract = new Contract(params.raffleContract);
    const operation = contract.call(
      'provide_randomness',
      nativeToScVal(params.randomSeed, { type: 'u64' }),
      nativeToScVal(Buffer.from(params.publicKey), { type: 'bytes' }),
      nativeToScVal(Buffer.from(params.proof), { type: 'bytes' }),
      nativeToScVal(params.requestId, { type: 'u64' }),
    );

    const fee = String(BigInt(100000) * BigInt(2 ** this.feeBumpCount));
    const timeout = 300 * this.timeoutMultiplier;

    let tx = new TransactionBuilder(sourceAccount, {
      fee,
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(operation)
      .setTimeout(timeout)
      .build();

    const simulated = await this.server.simulateTransaction(tx);
    if (SorobanRpc.Api.isSimulationError(simulated)) {
      throw new Error(`Simulation failed: ${JSON.stringify(simulated)}`);
    }

    const prepared = SorobanRpc.assembleTransaction(tx, simulated).build();
    prepared.sign(keypair);

    const sendResult = await this.server.sendTransaction(prepared);
    if (sendResult.status === 'ERROR') {
      throw new Error(
        `Send failed: ${sendResult.errorResult?.toXDR('base64') ?? 'unknown error'}`,
      );
    }

    const hash = sendResult.hash;
    const status = await this.pollTransaction(hash);

    if (status.status === SorobanRpc.Api.GetTransactionStatus.SUCCESS) {
      this.sequenceCache = String(BigInt(sequence) + 1n);
      console.log(`provide_randomness confirmed: ${hash}`);
      return hash;
    }

    if (status.status === SorobanRpc.Api.GetTransactionStatus.FAILED) {
      throw new Error(`Transaction failed on-chain: ${hash}`);
    }

    throw new Error(`Transaction did not confirm: ${hash}`);
  }

  private async pollTransaction(
    hash: string,
    maxAttempts = 30,
    intervalMs = 2000,
  ): Promise<SorobanRpc.Api.GetTransactionResponse> {
    for (let i = 0; i < maxAttempts; i++) {
      const result = await this.server.getTransaction(hash);
      if (result.status !== SorobanRpc.Api.GetTransactionStatus.NOT_FOUND) {
        return result;
      }
      await this.sleep(intervalMs);
    }
    throw new Error(`TxTooLate: transaction ${hash} not confirmed within timeout`);
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
}
