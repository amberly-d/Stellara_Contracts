import { TypeOrmModuleOptions } from '@nestjs/typeorm';

export type DatabaseConnectionOptions = {
  host: string;
  port: number;
  username: string;
  password?: string;
  database: string;
};

export const buildTypeOrmOptions = (
  config: DatabaseConnectionOptions,
): TypeOrmModuleOptions => ({
  type: 'postgres',
  host: config.host,
  port: config.port,
  username: config.username,
  password: config.password,
  database: config.database,
  synchronize: false,
  logging: false,
  entities: ['dist/**/*.entity{.ts,.js}'],
  migrations: ['dist/database/migrations/*{.ts,.js}'],
  extra: {
    max: 20,
    min: 5,
    idleTimeoutMillis: 30000,
  },
  retryAttempts: 5,
  retryDelay: 3000,
});
