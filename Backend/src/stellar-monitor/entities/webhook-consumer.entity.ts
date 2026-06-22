import {
  Entity,
  PrimaryGeneratedColumn,
  Column,
  CreateDateColumn,
  UpdateDateColumn,
  Index,
} from 'typeorm';
import { ConsumerStatus } from '../types/stellar.types';

@Entity('webhook_consumers')
export class WebhookConsumer {
  @PrimaryGeneratedColumn('uuid')
  id: string;

  @Column({ length: 100 })
  name: string;

  @Column({ length: 500 })
  @Index()
  url: string;

  @Column({ length: 100, nullable: true })
  secret: string;

  @Column({
    type: 'varchar',
    length: 20,
    default: ConsumerStatus.ACTIVE,
  })
  @Index()
  status: ConsumerStatus;

  @Column({ default: 5 })
  maxRetries: number;

  @Column({ default: 5000 }) // 5 seconds timeout
  timeoutMs: number;

  @Column({ default: true })
  @Index()
  isActive: boolean;

  @CreateDateColumn()
  createdAt: Date;

  @UpdateDateColumn()
  updatedAt: Date;

  @Column({ type: 'simple-json', nullable: true })
  metadata: Record<string, any>;

  @Column({ nullable: true })
  lastDeliveryAttempt?: Date;

  @Column({ nullable: true })
  lastDeliverySuccess?: Date;

  @Column({ default: 0 })
  totalDeliveries: number;

  @Column({ default: 0 })
  failedDeliveries: number;

  // Number of delivery attempts made for the most recent in-flight event.
  // Reset to 0 on a successful delivery; incremented on every retry.
  @Column({ default: 0 })
  deliveryAttempts: number;

  // Last error message recorded for a failed/dead-lettered delivery.
  @Column({ type: 'text', nullable: true })
  lastError?: string;
}
