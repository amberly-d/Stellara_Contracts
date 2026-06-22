import { Module } from '@nestjs/common';
import { TypeOrmModule } from '@nestjs/typeorm';
import { StellarMonitorModule } from '../stellar-monitor/stellar-monitor.module';
import { HealthController } from './health.controller';
import { DatabaseHealthIndicator } from './database-health.indicator';

@Module({
  imports: [StellarMonitorModule, TypeOrmModule.forFeature([])],
  controllers: [HealthController],
  providers: [DatabaseHealthIndicator],
})
export class HealthModule {}
