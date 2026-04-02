/**
 * Internal error used to short-circuit parser code before transport wraps it
 * into one public contract error value.
 */
export class ContractViolationError extends Error {
  public readonly path: string;

  public constructor(path: string, message: string) {
    super(message);
    this.name = "ContractViolationError";
    this.path = path;
  }
}

/**
 * Throw one internal contract-violation error.
 */
export function failContract(path: string, message: string): never {
  throw new ContractViolationError(path, message);
}
