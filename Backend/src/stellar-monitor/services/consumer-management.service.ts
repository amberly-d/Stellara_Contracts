import {
  Injectable,
  Logger,
  NotFoundException,
  BadRequestException,
} from '@nestjs/common';
import { InjectRepository } from '@nestjs/typeorm';
import { Repository } from 'typeorm';
import { WebhookConsumer } from '../entities/webhook-consumer.entity';
import { CreateConsumerDto } from '../dto/create-consumer.dto';
import { UpdateConsumerDto } from '../dto/update-consumer.dto';
import { ConsumerStatus } from '../types/stellar.types';

@Injectable()
export class ConsumerManagementService {
  private readonly logger = new Logger(ConsumerManagementService.name);

  constructor(
    @InjectRepository(WebhookConsumer)
    private readonly consumerRepository: Repository<WebhookConsumer>,
  ) {}

  async createConsumer(createDto: CreateConsumerDto): Promise<WebhookConsumer> {
    try {
      // Check if consumer with same URL already exists
      const existingConsumer = await this.consumerRepository.findOne({
        where: { url: createDto.url },
      });

      if (existingConsumer) {
        throw new BadRequestException('Consumer with this URL already exists');
      }

      const consumer = this.consumerRepository.create({
        ...createDto,
        status: ConsumerStatus.ACTIVE,
      });

      const savedConsumer = await this.consumerRepository.save(consumer);
      this.logger.log(
        `Created webhook consumer ${savedConsumer.id} (${savedConsumer.name})`,
      );
      return savedConsumer;
    } catch (error) {
      this.logger.error(
        `Failed to create consumer: ${error.message}`,
        error.stack,
      );
      throw error;
    }
  }

  async getAllConsumers(
    activeOnly: boolean = false,
  ): Promise<WebhookConsumer[]> {
    const queryBuilder = this.consumerRepository.createQueryBuilder('consumer');

    if (activeOnly) {
      queryBuilder.where('consumer.isActive = :isActive', { isActive: true });
    }

    return queryBuilder.orderBy('consumer.createdAt', 'DESC').getMany();
  }

  async getConsumerById(id: string): Promise<WebhookConsumer> {
    const consumer = await this.consumerRepository.findOne({ where: { id } });
    if (!consumer) {
      throw new NotFoundException(`Consumer with ID ${id} not found`);
    }
    return consumer;
  }

  async updateConsumer(
    id: string,
    updateDto: UpdateConsumerDto,
  ): Promise<WebhookConsumer> {
    const consumer = await this.getConsumerById(id);

    // Check if URL is being updated and conflicts with existing consumer
    if (updateDto.url && updateDto.url !== consumer.url) {
      const existingConsumer = await this.consumerRepository.findOne({
        where: { url: updateDto.url },
      });

      if (existingConsumer && existingConsumer.id !== id) {
        throw new BadRequestException(
          'Another consumer with this URL already exists',
        );
      }
    }

    Object.assign(consumer, updateDto);
    const updatedConsumer = await this.consumerRepository.save(consumer);

    this.logger.log(`Updated consumer ${id} (${updatedConsumer.name})`);
    return updatedConsumer;
  }

  async deleteConsumer(id: string): Promise<void> {
    const consumer = await this.getConsumerById(id);
    await this.consumerRepository.remove(consumer);
    this.logger.log(`Deleted consumer ${id} (${consumer.name})`);
  }

  async activateConsumer(id: string): Promise<WebhookConsumer> {
    const consumer = await this.getConsumerById(id);
    consumer.isActive = true;
    consumer.status = ConsumerStatus.ACTIVE;
    const updatedConsumer = await this.consumerRepository.save(consumer);

    this.logger.log(`Activated consumer ${id}`);
    return updatedConsumer;
  }

  async deactivateConsumer(id: string): Promise<WebhookConsumer> {
    const consumer = await this.getConsumerById(id);
    consumer.isActive = false;
    consumer.status = ConsumerStatus.INACTIVE;
    const updatedConsumer = await this.consumerRepository.save(consumer);

    this.logger.log(`Deactivated consumer ${id}`);
    return updatedConsumer;
  }

  async suspendConsumer(id: string, reason?: string): Promise<WebhookConsumer> {
    const consumer = await this.getConsumerById(id);
    consumer.isActive = false;
    consumer.status = ConsumerStatus.SUSPENDED;

    if (reason) {
      consumer.metadata = {
        ...consumer.metadata,
        suspensionReason: reason,
        suspendedAt: new Date().toISOString(),
      };
    }

    const updatedConsumer = await this.consumerRepository.save(consumer);
    this.logger.log(`Suspended consumer ${id}${reason ? ` (${reason})` : ''}`);
    return updatedConsumer;
  }

  async getActiveConsumers(): Promise<WebhookConsumer[]> {
    return this.consumerRepository.find({
      where: {
        isActive: true,
        status: ConsumerStatus.ACTIVE,
      },
      order: { createdAt: 'ASC' },
    });
  }

  async updateDeliveryStats(
    consumerId: string,
    isSuccess: boolean,
    deliveryTime?: Date,
  ): Promise<WebhookConsumer> {
    const consumer = await this.getConsumerById(consumerId);

    consumer.totalDeliveries += 1;
    consumer.lastDeliveryAttempt = new Date();

    if (isSuccess) {
      consumer.lastDeliverySuccess = deliveryTime || new Date();
      consumer.failedDeliveries = Math.max(0, consumer.failedDeliveries - 1); // Reduce failure count on success
    } else {
      consumer.failedDeliveries += 1;

      // Auto-suspend if too many failures
      if (consumer.failedDeliveries > 10) {
        consumer.isActive = false;
        consumer.status = ConsumerStatus.SUSPENDED;
        consumer.metadata = {
          ...consumer.metadata,
          autoSuspended: true,
          autoSuspensionReason: 'Too many consecutive failures',
          autoSuspendedAt: new Date().toISOString(),
        };
        this.logger.warn(
          `Auto-suspended consumer ${consumerId} due to excessive failures`,
        );
      }
    }

    return this.consumerRepository.save(consumer);
  }

  /**
   * Records delivery progress (retry attempts and last error) on a consumer.
   * Pass `attempts = 0` and `lastError = null` to reset after a success.
   */
  async recordDeliveryProgress(
    consumerId: string,
    attempts: number,
    lastError: string | null,
  ): Promise<WebhookConsumer> {
    const consumer = await this.getConsumerById(consumerId);
    consumer.deliveryAttempts = attempts;
    consumer.lastError = lastError ?? undefined;
    return this.consumerRepository.save(consumer);
  }

  async getConsumerHealthStats(consumerId: string): Promise<{
    successRate: number;
    recentFailures: number;
    avgResponseTime: number;
    lastDelivery: Date | null;
  }> {
    const consumer = await this.getConsumerById(consumerId);

    const successRate =
      consumer.totalDeliveries > 0
        ? ((consumer.totalDeliveries - consumer.failedDeliveries) /
            consumer.totalDeliveries) *
          100
        : 100;

    return {
      successRate: parseFloat(successRate.toFixed(2)),
      recentFailures: consumer.failedDeliveries,
      avgResponseTime: 0, // Would need to track response times separately
      lastDelivery: consumer.lastDeliveryAttempt || null,
    };
  }

  async getStats(): Promise<{
    totalConsumers: number;
    activeConsumers: number;
    inactiveConsumers: number;
    suspendedConsumers: number;
    consumersByStatus: Record<ConsumerStatus, number>;
  }> {
    const totalConsumers = await this.consumerRepository.count();
    const activeConsumers = await this.consumerRepository.count({
      where: { isActive: true },
    });
    const inactiveConsumers = await this.consumerRepository.count({
      where: { status: ConsumerStatus.INACTIVE },
    });
    const suspendedConsumers = await this.consumerRepository.count({
      where: { status: ConsumerStatus.SUSPENDED },
    });

    const consumersByStatus: Record<ConsumerStatus, number> = {} as Record<
      ConsumerStatus,
      number
    >;
    for (const status of Object.values(ConsumerStatus)) {
      consumersByStatus[status] = await this.consumerRepository.count({
        where: { status },
      });
    }

    return {
      totalConsumers,
      activeConsumers,
      inactiveConsumers,
      suspendedConsumers,
      consumersByStatus,
    };
  }
}
