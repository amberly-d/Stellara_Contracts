/**
 * Base error class for queue job errors.
 * Subclasses signal whether a job should be retried or routed to the DLQ.
 */
export abstract class QueueJobError extends Error {
  abstract readonly retryable: boolean;
  constructor(
    message: string,
    public readonly context?: Record<string, unknown>,
  ) {
    super(message);
    this.name = new.target.name;
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/** Transient failures that should be retried (network timeouts, rate limits). */
export class TransientError extends QueueJobError {
  readonly retryable = true;
}

/** Permanent failures that should be routed directly to the DLQ. */
export class PermanentError extends QueueJobError {
  readonly retryable = false;
}

/** Validation failures (bad input) — never retryable. */
export class ValidationError extends QueueJobError {
  readonly retryable = false;
}
