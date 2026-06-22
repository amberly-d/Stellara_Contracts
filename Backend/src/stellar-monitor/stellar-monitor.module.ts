import { Module } from '@nestjs/common';
import { TypeOrmModule } from '@nestjs/typeorm';
import { BullModule } from '@nestjs/bull';
import { StellarMonitorController } from './controllers/stellar-monitor.controller';
import { StellarEventMonitorService } from './services/stellar-event-monitor.service';
import { WebhookDeliveryService } from './services/webhook-delivery.service';
import { EventStorageService } from './services/event-storage.service';
import { ConsumerManagementService } from './services/consumer-management.service';
import { WebhookConsumer } from './entities/webhook-consumer.entity';
import { StellarEvent } from './entities/stellar-event.entity';
import { WebhookDeliveryProcessor } from './processors/webhook-delivery.processor';

@Module({
  imports: [
    TypeOrmModule.forFeature([WebhookConsumer, StellarEvent]),
    BullModule.registerQueue({ name: 'webhook-delivery' }),
  ],
  controllers: [StellarMonitorController],
  providers: [
    StellarEventMonitorService,
    WebhookDeliveryService,
    WebhookDeliveryProcessor,
    EventStorageService,
    ConsumerManagementService,
  ],
  exports: [
    StellarEventMonitorService,
    WebhookDeliveryService,
    EventStorageService,
    ConsumerManagementService,
  ],
})
export class StellarMonitorModule {}
