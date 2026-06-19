import {
  Entity,
  Column,
  PrimaryGeneratedColumn,
  CreateDateColumn,
} from 'typeorm';

export abstract class AuditLogEntry {
  @PrimaryGeneratedColumn('uuid')
  id: string;

  @Column()
  action_type: string;

  @Column()
  actor_id: string;

  @Column({ nullable: true })
  entity_id?: string;

  @Column('jsonb', { nullable: true })
  metadata?: Record<string, any>;

  @CreateDateColumn()
  timestamp: Date;

  @Column({ default: '' })
  previousHash: string;

  @Column({ default: '' })
  hash: string;

  @Column({ default: '' })
  signature: string;
}

@Entity('audit_logs')
export class AuditLog extends AuditLogEntry {}

@Entity('audit_log_archives')
export class AuditLogArchive extends AuditLogEntry {
  @CreateDateColumn()
  archivedAt: Date;
}
