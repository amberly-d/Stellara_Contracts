import { Module } from '@nestjs/common';
import { BullModule } from '@nestjs/bull';
import { ConfigModule, ConfigService } from '@nestjs/config';
import { QueueService } from './services/queue.service';
import { DeployContractProcessor } from './processors/deploy-contract.processor';
import { ProcessTtsProcessor } from './processors/process-tts.processor';
import { IndexMarketNewsProcessor } from './processors/index-market-news.processor';
import { DeadLetterProcessor } from './processors/dead-letter.processor';
import { QueueAdminController } from './controllers/queue-admin.controller';
import { RedisModule } from '../redis/redis.module';
import { ObservabilityModule } from '../observability/observability.module';

@Module({
  imports: [
    BullModule.forRootAsync({
      imports: [ConfigModule],
      useFactory: (configService: ConfigService) => ({
        redis: {
          host: configService.get('REDIS_HOST') || 'localhost',
          port: configService.get('REDIS_PORT') || 6379,
          db: configService.get('REDIS_QUEUE_DB') || 1,
        },
        defaultJobOptions: {
          attempts: 3,
          backoff: {
            type: 'exponential',
            delay: 2000,
          },
          removeOnComplete: true,
        },
      }),
      inject: [ConfigService],
    }),
    BullModule.registerQueue(
      { name: 'deploy-contract' },
      { name: 'process-tts' },
      { name: 'index-market-news' },
      { name: 'failed-jobs' },
    ),
    RedisModule,
    ObservabilityModule,
  ],
  controllers: [QueueAdminController],
  providers: [
    QueueService,
    DeployContractProcessor,
    ProcessTtsProcessor,
    IndexMarketNewsProcessor,
    DeadLetterProcessor,
  ],
  exports: [QueueService],
})
export class QueueModule {}
