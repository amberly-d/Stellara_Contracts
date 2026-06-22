import {
  IsString,
  IsUrl,
  IsOptional,
  IsInt,
  Min,
  Max,
  IsBoolean,
  Length,
} from 'class-validator';
import { Transform } from 'class-transformer';
import { IsSafeWebhookUrl } from '../validators/is-safe-webhook-url.validator';

export class CreateConsumerDto {
  @IsString()
  @Length(1, 100)
  name: string;

  @IsUrl({ require_protocol: true })
  @IsSafeWebhookUrl()
  @Length(1, 500)
  url: string;

  @IsOptional()
  @IsString()
  @Length(1, 100)
  secret?: string;

  @IsOptional()
  @IsInt()
  @Min(0)
  @Max(20)
  @Transform(({ value }) => parseInt(value, 10))
  maxRetries?: number = 5;

  @IsOptional()
  @IsInt()
  @Min(1000)
  @Max(30000)
  @Transform(({ value }) => parseInt(value, 10))
  timeoutMs?: number = 5000;

  @IsOptional()
  @IsBoolean()
  isActive?: boolean = true;
}
