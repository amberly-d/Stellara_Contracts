import { Module } from '@nestjs/common';
import { TypeOrmModule } from '@nestjs/typeorm';
import { BullModule } from '@nestjs/bull';
import { VoiceGateway } from './voice.gateway';
import { VoiceSessionService } from './services/voice-session.service';
import { ConversationStateMachineService } from './services/conversation-state-machine.service';
import { StreamingResponseService } from './services/streaming-response.service';
import { SessionCleanupService } from './services/session-cleanup.service';
import { LlmService } from './services/llm.service';
import { QuotaService } from './services/quota.service';
import { LlmCacheService } from './services/llm-cache.service';
import { VoiceJob } from './entities/voice-job.entity';
import { VoiceSession } from './entities/voice-session.entity';
import { AiUsageQuota } from '../ai/quota/quota.entity';
import { JwtService } from '@nestjs/jwt';
import { WsJwtAuthGuard } from '../auth/guards/ws-jwt-auth.guard';
import { RedisModule } from '../redis/redis.module';

@Module({
  imports: [
    RedisModule,
    TypeOrmModule.forFeature([VoiceSession, VoiceJob, AiUsageQuota]),
    BullModule.registerQueue({
      name: 'voice-processing',
    }),
  ],
  providers: [
    VoiceGateway,
    VoiceSessionService,
    ConversationStateMachineService,
    StreamingResponseService,
    SessionCleanupService,
    LlmService,
    QuotaService,
    LlmCacheService,
    JwtService,
    WsJwtAuthGuard,
  ],
  exports: [
    VoiceSessionService,
    ConversationStateMachineService,
    StreamingResponseService,
    LlmService,
    QuotaService,
    LlmCacheService,
  ],
})
export class VoiceModule {}