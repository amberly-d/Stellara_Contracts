import { Injectable, Logger } from '@nestjs/common';
import { InjectQueue } from '@nestjs/bull';
import type { Queue } from 'bull';
import axios, { AxiosInstance, AxiosRequestConfig } from 'axios';
import { WebhookConsumer } from '../entities/webhook-consumer.entity';
import { StellarEvent } from '../entities/stellar-event.entity';
import { ConsumerManagementService } from './consumer-management.service';
import { EventStorageService } from './event-storage.service';
import { DeliveryStatus, EventType } from '../types/stellar.types';
import { WebhookDeliveryJobData } from '../processors/webhook-delivery.processor';

interface DeliveryResult {
  success: boolean;
  statusCode?: number;
  errorMessage?: string;
  responseTime?: number;
}

@Injectable()
export class WebhookDeliveryService {
  private readonly logger = new Logger(WebhookDeliveryService.name);
  private readonly httpClient: AxiosInstance;

  constructor(
    @InjectQueue('webhook-delivery') private readonly deliveryQueue: Queue<WebhookDeliveryJobData>,
    private readonly consumerManagementService: ConsumerManagementService,
    private readonly eventStorageService: EventStorageService,
  ) {
    this.httpClient = axios.create({
      timeout: 10000,
      headers: {
        'Content-Type': 'application/json',
        'User-Agent': 'Stellar-Monitor/1.0',
      },
    });
  }

  async queueEventForDelivery(event: StellarEvent): Promise<void> {
    const activeConsumers =
      await this.consumerManagementService.getActiveConsumers();

    if (activeConsumers.length === 0) {
      this.logger.debug('No active consumers, marking event as processed');
      await this.eventStorageService.markEventAsProcessed(event.id);
      return;
    }

    for (const consumer of activeConsumers) {
      await this.deliveryQueue.add(
        { event, consumer },
        {
          attempts: consumer.maxRetries,
          backoff: { type: 'exponential', delay: 1000 },
          removeOnComplete: true,
        },
      );
      this.logger.debug(`Queued event ${event.id} for consumer ${consumer.id}`);
    }
  }

  /** Called by WebhookDeliveryProcessor — delivers one event to one consumer. */
  async deliverEventToConsumer(
    event: StellarEvent,
    consumer: WebhookConsumer,
  ): Promise<void> {
    const startTime = Date.now();

    try {
      const freshConsumer =
        await this.consumerManagementService.getConsumerById(consumer.id);
      if (!freshConsumer.isActive) {
        this.logger.debug(
          `Skipping delivery to inactive consumer ${consumer.id}`,
        );
        return;
      }

      const result = await this.attemptDelivery(event, freshConsumer);
      const responseTime = Date.now() - startTime;

      if (result.success) {
        this.logger.log(
          `Successfully delivered event ${event.id} to consumer ${consumer.id}`,
        );
        await this.handleSuccessfulDelivery(event, consumer, responseTime);
      } else {
        this.logger.warn(
          `Failed to deliver event ${event.id} to consumer ${consumer.id}: ${result.errorMessage}`,
        );
        await this.handleFailedDelivery(event, consumer, result, responseTime);
        // Re-throw so Bull can apply retry/backoff
        throw new Error(result.errorMessage || 'Delivery failed');
      }
    } catch (error) {
      const responseTime = Date.now() - startTime;
      this.logger.error(
        `Exception during delivery to consumer ${consumer.id}: ${error.message}`,
        error.stack,
      );
      await this.handleFailedDelivery(
        event,
        consumer,
        { success: false, errorMessage: error.message },
        responseTime,
      );
      throw error;
    }
  }

  private async attemptDelivery(
    event: StellarEvent,
    consumer: WebhookConsumer,
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

    const config: AxiosRequestConfig = {
      timeout: consumer.timeoutMs,
      headers: {
        'X-Stellar-Event-ID': event.id,
        'X-Stellar-Event-Type': event.eventType,
        'X-Delivery-Attempt': (event.deliveryAttempts + 1).toString(),
      },
    };

    if (consumer.secret) {
      const signature = this.generateSignature(
        JSON.stringify(payload),
        consumer.secret,
      );
      config.headers = { ...config.headers, 'X-Signature': signature };
    }

    try {
      const response = await this.httpClient.post(consumer.url, payload, config);
      return { success: true, statusCode: response.status };
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
    await this.eventStorageService.updateEventStatus(
      event.id,
      DeliveryStatus.DELIVERED,
      consumer.id,
    );
    await this.consumerManagementService.updateDeliveryStats(consumer.id, true);

    const activeConsumers =
      await this.consumerManagementService.getActiveConsumers();
    const deliveredCount = (event.deliveredTo?.length || 0) + 1;
    if (deliveredCount >= activeConsumers.length) {
      await this.eventStorageService.markEventAsProcessed(event.id);
    }
  }

  private async handleFailedDelivery(
    event: StellarEvent,
    consumer: WebhookConsumer,
    result: DeliveryResult,
    responseTime: number,
  ): Promise<void> {
    await this.consumerManagementService.updateDeliveryStats(consumer.id, false);

    const attemptNumber = event.deliveryAttempts + 1;
    if (attemptNumber >= consumer.maxRetries) {
      await this.eventStorageService.updateEventStatus(
        event.id,
        DeliveryStatus.FAILED,
        consumer.id,
        result.errorMessage,
      );
      this.logger.warn(
        `Max retries reached for event ${event.id} to consumer ${consumer.id}`,
      );
    }
  }

  private generateSignature(payload: string, secret: string): string {
    return require('crypto')
      .createHmac('sha256', secret)
      .update(payload)
      .digest('hex');
  }

  async deliverTestEvent(consumerId: string): Promise<DeliveryResult> {
    const consumer =
      await this.consumerManagementService.getConsumerById(consumerId);

    if (!consumer.isActive) {
      return { success: false, errorMessage: 'Consumer is not active' };
    }

    const testEvent: Partial<StellarEvent> = {
      id: 'test-' + Date.now(),
      eventType: EventType.PAYMENT,
      ledgerSequence: 123456,
      timestamp: new Date(),
      transactionHash: 'test-transaction-hash',
      sourceAccount: 'test-source-account',
      payload: { test: true, message: 'This is a test event' },
    };

    return this.attemptDelivery(testEvent as StellarEvent, consumer);
  }

  async getQueueSize(): Promise<number> {
    return this.deliveryQueue.count();
  }

  async getDeliveryStats(): Promise<{
    queueSize: number;
    activeConsumers: number;
    pendingEvents: number;
  }> {
    const [activeConsumers, pendingEvents, queueSize] = await Promise.all([
      this.consumerManagementService.getActiveConsumers(),
      this.eventStorageService.getPendingEvents(),
      this.deliveryQueue.count(),
    ]);

    return {
      queueSize,
      activeConsumers: activeConsumers.length,
      pendingEvents: pendingEvents.length,
    };
  }
}
