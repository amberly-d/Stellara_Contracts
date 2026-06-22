import { Injectable, Logger } from '@nestjs/common';
import { createHmac } from 'crypto';
import axios, { AxiosInstance, AxiosRequestConfig } from 'axios';
import { WebhookConsumer } from '../entities/webhook-consumer.entity';
import { StellarEvent } from '../entities/stellar-event.entity';
import { ConsumerManagementService } from './consumer-management.service';
import { EventStorageService } from './event-storage.service';
import { DeliveryStatus, EventType } from '../types/stellar.types';
import { validateWebhookUrl } from '../utils/ssrf.util';

interface DeliveryResult {
  success: boolean;
  statusCode?: number;
  errorMessage?: string;
  responseTime?: number;
}

interface QueueItem {
  event: StellarEvent;
  consumer: WebhookConsumer;
  attempt: number; // 1-based attempt counter
}

@Injectable()
export class WebhookDeliveryService {
  private readonly logger = new Logger(WebhookDeliveryService.name);
  private readonly httpClient: AxiosInstance;
  private readonly deliveryQueue: QueueItem[] = [];
  private isProcessingQueue = false;

  /** Exponential backoff schedule between attempts: 1s, 5s, 30s, 5m, 30m. */
  static readonly RETRY_DELAYS_MS = [1000, 5000, 30000, 300000, 1800000];
  /** Max delivery attempts before an event is moved to the dead-letter state. */
  static readonly MAX_ATTEMPTS = 5;
  /** Outbound HMAC signature header name. */
  static readonly SIGNATURE_HEADER = 'X-Stellara-Signature';

  // Keys (`${eventId}:${consumerId}`) already delivered — guarantees idempotency
  // so a duplicate queue entry or retry can never double-deliver.
  private readonly delivered = new Set<string>();
  // Keys currently queued/in-flight — prevents queueing the same pair twice.
  private readonly inFlight = new Set<string>();

  constructor(
    private readonly consumerManagementService: ConsumerManagementService,
    private readonly eventStorageService: EventStorageService,
  ) {
    this.httpClient = axios.create({
      timeout: 10000, // 10 second default timeout
      headers: {
        'Content-Type': 'application/json',
        'User-Agent': 'Stellar-Monitor/1.0',
      },
    });

    // Setup automatic queue processing
    setInterval(() => this.processQueue(), 1000); // Process queue every second
  }

  private deliveryKey(eventId: string, consumerId: string): string {
    return `${eventId}:${consumerId}`;
  }

  async queueEventForDelivery(event: StellarEvent): Promise<void> {
    const activeConsumers =
      await this.consumerManagementService.getActiveConsumers();

    if (activeConsumers.length === 0) {
      this.logger.debug('No active consumers, marking event as processed');
      await this.eventStorageService.markEventAsProcessed(event.id);
      return;
    }

    // Add event to queue for each active consumer (deduplicated).
    for (const consumer of activeConsumers) {
      const key = this.deliveryKey(event.id, consumer.id);
      if (this.delivered.has(key) || this.inFlight.has(key)) {
        this.logger.debug(
          `Skipping duplicate queue for event ${event.id} / consumer ${consumer.id}`,
        );
        continue;
      }
      this.inFlight.add(key);
      this.deliveryQueue.push({ event, consumer, attempt: 1 });
      this.logger.debug(`Queued event ${event.id} for consumer ${consumer.id}`);
    }

    // Start processing if not already running
    if (!this.isProcessingQueue) {
      this.processQueue();
    }
  }

  private async processQueue(): Promise<void> {
    if (this.isProcessingQueue || this.deliveryQueue.length === 0) {
      return;
    }

    this.isProcessingQueue = true;
    this.logger.debug(
      `Processing delivery queue with ${this.deliveryQueue.length} items`,
    );

    try {
      // Process items in batches to avoid overwhelming the system
      const batchSize = 10;
      const batch = this.deliveryQueue.splice(0, batchSize);

      await Promise.all(
        batch.map(async ({ event, consumer, attempt }) => {
          await this.deliverEventToConsumer(event, consumer, attempt);
        }),
      );
    } catch (error) {
      this.logger.error(
        `Error processing delivery queue: ${error.message}`,
        error.stack,
      );
    } finally {
      this.isProcessingQueue = false;
    }
  }

  private async deliverEventToConsumer(
    event: StellarEvent,
    consumer: WebhookConsumer,
    attempt: number,
  ): Promise<void> {
    const startTime = Date.now();
    const key = this.deliveryKey(event.id, consumer.id);

    // Idempotency: never deliver the same event to the same consumer twice.
    if (this.delivered.has(key)) {
      this.logger.debug(
        `Event ${event.id} already delivered to consumer ${consumer.id}, skipping`,
      );
      return;
    }

    try {
      // Check if consumer is still active (might have been deactivated during queue processing)
      const freshConsumer =
        await this.consumerManagementService.getConsumerById(consumer.id);
      if (!freshConsumer.isActive) {
        this.logger.debug(
          `Skipping delivery to inactive consumer ${consumer.id}`,
        );
        this.inFlight.delete(key);
        return;
      }

      // SSRF protection: validate the (possibly re-pointed) URL right before
      // calling it. An unsafe URL is permanently undeliverable — dead-letter it
      // immediately rather than retrying.
      try {
        await validateWebhookUrl(freshConsumer.url);
      } catch (ssrfError) {
        this.logger.warn(
          `Refusing delivery to unsafe URL for consumer ${consumer.id}: ${ssrfError.message}`,
        );
        await this.moveToDeadLetter(event, consumer, attempt, ssrfError.message);
        return;
      }

      const result = await this.attemptDelivery(event, freshConsumer, attempt);

      const responseTime = Date.now() - startTime;

      if (result.success) {
        this.delivered.add(key);
        this.inFlight.delete(key);
        this.logger.log(
          `Successfully delivered event ${event.id} to consumer ${consumer.id}`,
        );
        await this.handleSuccessfulDelivery(event, consumer, responseTime);
      } else {
        this.logger.warn(
          `Failed to deliver event ${event.id} to consumer ${consumer.id} (attempt ${attempt}): ${result.errorMessage}`,
        );
        await this.handleFailedDelivery(event, consumer, result, attempt);
      }
    } catch (error) {
      this.logger.error(
        `Exception during delivery to consumer ${consumer.id}: ${error.message}`,
        error.stack,
      );
      await this.handleFailedDelivery(
        event,
        consumer,
        {
          success: false,
          errorMessage: error.message,
        },
        attempt,
      );
    }
  }

  private async attemptDelivery(
    event: StellarEvent,
    consumer: WebhookConsumer,
    attempt: number,
  ): Promise<DeliveryResult> {
    const payload = {
      id: event.id,
      eventType: event.eventType,
      ledgerSequence: event.ledgerSequence,
      timestamp: event.timestamp.toISOString(),
      transactionHash: event.transactionHash,
      sourceAccount: event.sourceAccount,
      payload: event.payload,
      deliveredAt: new Date().toISOString(),
    };

    const body = JSON.stringify(payload);

    const config: AxiosRequestConfig = {
      timeout: consumer.timeoutMs,
      headers: {
        'X-Stellar-Event-ID': event.id,
        'X-Stellar-Event-Type': event.eventType,
        'X-Delivery-Attempt': attempt.toString(),
        // Idempotency key lets consumers dedupe on their side too.
        'X-Idempotency-Key': this.deliveryKey(event.id, consumer.id),
      },
    };

    // Add HMAC-SHA256 signature if consumer has a secret so the consumer can
    // verify the payload originated from us and was not tampered with.
    if (consumer.secret) {
      config.headers = {
        ...config.headers,
        [WebhookDeliveryService.SIGNATURE_HEADER]: this.generateSignature(
          body,
          consumer.secret,
        ),
      };
    }

    try {
      const response = await this.httpClient.post(consumer.url, body, config);

      return {
        success: true,
        statusCode: response.status,
      };
    } catch (error) {
      return {
        success: false,
        statusCode: error.response?.status,
        errorMessage: error.message,
      };
    }
  }

  private async handleSuccessfulDelivery(
    event: StellarEvent,
    consumer: WebhookConsumer,
    responseTime: number,
  ): Promise<void> {
    // Update event status
    await this.eventStorageService.updateEventStatus(
      event.id,
      DeliveryStatus.DELIVERED,
      consumer.id,
    );

    // Update consumer stats and reset the in-flight attempt counter.
    await this.consumerManagementService.updateDeliveryStats(consumer.id, true);
    await this.consumerManagementService.recordDeliveryProgress(
      consumer.id,
      0,
      null,
    );

    // Mark event as processed if delivered to all consumers
    const activeConsumers =
      await this.consumerManagementService.getActiveConsumers();
    const deliveredCount = (event.deliveredTo?.length || 0) + 1; // +1 for current delivery

    if (deliveredCount >= activeConsumers.length) {
      await this.eventStorageService.markEventAsProcessed(event.id);
    }
  }

  private async handleFailedDelivery(
    event: StellarEvent,
    consumer: WebhookConsumer,
    result: DeliveryResult,
    attempt: number,
  ): Promise<void> {
    // Update consumer stats and record the attempt/error on the consumer.
    await this.consumerManagementService.updateDeliveryStats(
      consumer.id,
      false,
    );
    await this.consumerManagementService.recordDeliveryProgress(
      consumer.id,
      attempt,
      result.errorMessage ?? null,
    );

    if (attempt >= WebhookDeliveryService.MAX_ATTEMPTS) {
      // Max attempts reached — move to dead-letter.
      await this.moveToDeadLetter(
        event,
        consumer,
        attempt,
        result.errorMessage,
      );
      return;
    }

    // Schedule retry with exponential backoff.
    const delay = this.calculateBackoffDelay(attempt);
    this.logger.debug(
      `Scheduling retry ${attempt + 1}/${WebhookDeliveryService.MAX_ATTEMPTS} for event ${event.id} in ${delay}ms`,
    );

    await this.eventStorageService.updateEventStatus(
      event.id,
      DeliveryStatus.RETRYING,
      consumer.id,
      result.errorMessage,
    );

    setTimeout(() => {
      this.deliveryQueue.push({ event, consumer, attempt: attempt + 1 });
      if (!this.isProcessingQueue) {
        this.processQueue();
      }
    }, delay);
  }

  private async moveToDeadLetter(
    event: StellarEvent,
    consumer: WebhookConsumer,
    attempt: number,
    errorMessage?: string,
  ): Promise<void> {
    this.inFlight.delete(this.deliveryKey(event.id, consumer.id));

    await this.eventStorageService.updateEventStatus(
      event.id,
      DeliveryStatus.DEAD_LETTER,
      consumer.id,
      errorMessage,
    );

    this.logger.error(
      `Event ${event.id} moved to dead-letter for consumer ${consumer.id} after ${attempt} attempt(s): ${errorMessage ?? 'unknown error'}`,
    );
  }

  /**
   * Returns the backoff delay (ms) to wait before the next attempt.
   * `attempt` is the 1-based number of the attempt that just failed.
   */
  private calculateBackoffDelay(attempt: number): number {
    const delays = WebhookDeliveryService.RETRY_DELAYS_MS;
    const index = Math.min(attempt - 1, delays.length - 1);
    return delays[index];
  }

  private generateSignature(payload: string, secret: string): string {
    return createHmac('sha256', secret).update(payload).digest('hex');
  }

  async deliverTestEvent(consumerId: string): Promise<DeliveryResult> {
    const consumer =
      await this.consumerManagementService.getConsumerById(consumerId);

    if (!consumer.isActive) {
      return {
        success: false,
        errorMessage: 'Consumer is not active',
      };
    }

    // Validate the URL before sending a test event too.
    try {
      await validateWebhookUrl(consumer.url);
    } catch (error) {
      return {
        success: false,
        errorMessage: error.message,
      };
    }

    const testEvent: Partial<StellarEvent> = {
      id: 'test-' + Date.now(),
      eventType: EventType.PAYMENT,
      ledgerSequence: 123456,
      timestamp: new Date(),
      transactionHash: 'test-transaction-hash',
      sourceAccount: 'test-source-account',
      payload: {
        test: true,
        message: 'This is a test event',
      },
    };

    return this.attemptDelivery(testEvent as StellarEvent, consumer, 1);
  }

  getQueueSize(): number {
    return this.deliveryQueue.length;
  }

  async getDeliveryStats(): Promise<{
    queueSize: number;
    activeConsumers: number;
    pendingEvents: number;
  }> {
    const activeConsumers =
      await this.consumerManagementService.getActiveConsumers();
    const pendingEvents = await this.eventStorageService.getPendingEvents();

    return {
      queueSize: this.deliveryQueue.length,
      activeConsumers: activeConsumers.length,
      pendingEvents: pendingEvents.length,
    };
  }
}
