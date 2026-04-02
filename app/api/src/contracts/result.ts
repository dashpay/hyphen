/**
 * Public Result surface used by the Hyphen API client.
 */
export interface Result<TValue, TError> {
  /**
   * Read the current ok value, or `undefined` when the result is an error.
   */
  value(): TValue | undefined;

  /**
   * Read the current error value, or `undefined` when the result is ok.
   */
  error(): TError | undefined;

  /**
   * Read the ok value, or return one caller-provided fallback.
   */
  valueOr(fallback: TValue): TValue;

  /**
   * Read the error value, or return one caller-provided fallback.
   */
  errorOr(fallback: TError): TError;

  /**
   * Chain one follow-up Result-producing operation on the ok branch only.
   */
  andThen<TNext>(fn: (value: TValue) => Result<TNext, TError>): Result<TNext, TError>;

  /**
   * Transform the ok branch while preserving the current error type.
   */
  transform<TNext>(fn: (value: TValue) => TNext): Result<TNext, TError>;

  /**
   * Recover from the current error branch with one Result-producing callback.
   */
  orElse<TNextError>(fn: (error: TError) => Result<TValue, TNextError>): Result<TValue, TNextError>;

  /**
   * Transform the error branch while preserving the current ok type.
   */
  transformError<TNextError>(fn: (error: TError) => TNextError): Result<TValue, TNextError>;
}

class OkResult<TValue, TError> implements Result<TValue, TError> {
  public readonly valueData: TValue;

  public constructor(value: TValue) {
    this.valueData = value;
  }

  public value(): TValue {
    return this.valueData;
  }

  public error(): undefined {
    return undefined;
  }

  public valueOr(fallback: TValue): TValue {
    void fallback;
    return this.valueData;
  }

  public errorOr(fallback: TError): TError {
    return fallback;
  }

  public andThen<TNext>(fn: (value: TValue) => Result<TNext, TError>): Result<TNext, TError> {
    return fn(this.valueData);
  }

  public transform<TNext>(fn: (value: TValue) => TNext): Result<TNext, TError> {
    return ok(fn(this.valueData));
  }

  public orElse<TNextError>(fn: (error: TError) => Result<TValue, TNextError>): Result<TValue, TNextError> {
    void fn;
    return ok(this.valueData);
  }

  public transformError<TNextError>(fn: (error: TError) => TNextError): Result<TValue, TNextError> {
    void fn;
    return ok(this.valueData);
  }
}

class ErrResult<TValue, TError> implements Result<TValue, TError> {
  public readonly errorData: TError;

  public constructor(error: TError) {
    this.errorData = error;
  }

  public value(): undefined {
    return undefined;
  }

  public error(): TError {
    return this.errorData;
  }

  public valueOr(fallback: TValue): TValue {
    return fallback;
  }

  public errorOr(fallback: TError): TError {
    void fallback;
    return this.errorData;
  }

  public andThen<TNext>(fn: (value: TValue) => Result<TNext, TError>): Result<TNext, TError> {
    void fn;
    return err(this.errorData);
  }

  public transform<TNext>(fn: (value: TValue) => TNext): Result<TNext, TError> {
    void fn;
    return err(this.errorData);
  }

  public orElse<TNextError>(fn: (error: TError) => Result<TValue, TNextError>): Result<TValue, TNextError> {
    return fn(this.errorData);
  }

  public transformError<TNextError>(fn: (error: TError) => TNextError): Result<TValue, TNextError> {
    return err(fn(this.errorData));
  }
}

/**
 * Create one ok Result value.
 */
export function ok<TValue, TError = never>(value: TValue): Result<TValue, TError> {
  return new OkResult<TValue, TError>(value);
}

/**
 * Create one error Result value.
 */
export function err<TValue = never, TError = never>(error: TError): Result<TValue, TError> {
  return new ErrResult<TValue, TError>(error);
}
