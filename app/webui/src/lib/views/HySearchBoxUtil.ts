/**
 * Query classification for the search box's exact-lookup path.
 *
 * Extracted from `HySearchBox.svelte` so the rules can be unit-tested: the
 * ordering below is load-bearing and was wrong in a way no rendering test would
 * have caught.
 */

/** Largest value the daemon will parse a height into (`i32::MAX`). */
const MAX_BLOCK_HEIGHT = 2_147_483_647;

/** Shortest non-hash query worth an exact lookup. */
const MIN_EXACT_LOOKUP_LEN = 3;

/**
 * A decimal block height.
 *
 * This MUST be tested before {@link HySearchIsHexQuery}. Decimal digits are a
 * strict subset of the hex alphabet, so every height also satisfies the hex
 * test; checking hex first sent each one down the full-64-character-hash branch
 * and discarded it, which meant no numeric height ever reached
 * `/api/v1/search`. The daemon has always answered them — it probes the height
 * as a primary key before anything else and deliberately exempts it from the
 * four-character floor that applies to prefix queries — so the entire feature
 * was being dropped one layer above the request.
 *
 * The upper bound mirrors the daemon's own `query.parse::<i32>()` guard. There
 * is no lower bound beyond zero and no tip check: genesis is a real block, and
 * a height past the tip is a miss the daemon reports as an empty result rather
 * than an error.
 */
export function HySearchIsHeightQuery(value: string): boolean {
  if (!/^[0-9]+$/.test(value)) {
    return false;
  }
  const height = Number(value);
  return Number.isSafeInteger(height) && height >= 0 && height <= MAX_BLOCK_HEIGHT;
}

/** Whether the query is drawn entirely from the hex alphabet. */
export function HySearchIsHexQuery(value: string): boolean {
  return /^[0-9a-f]+$/i.test(value);
}

/** Whether the query is a complete 32-byte hash in hex. */
export function HySearchIsFullHash(value: string): boolean {
  return /^[0-9a-f]{64}$/i.test(value);
}

/**
 * Whether a query should be sent to the exact-lookup endpoint.
 *
 * Partial hex prefixes are deliberately withheld: the locally-scored suggestion
 * pool already covers them, so an exact lookup is reserved for queries that can
 * resolve to one object — a full hash, a block height, or a base58/free-text
 * term.
 */
export function HySearchShouldUseExactLookup(value: string): boolean {
  const trimmedValue = value.trim();
  if (trimmedValue.length === 0) {
    return false;
  }

  if (HySearchIsHeightQuery(trimmedValue)) {
    return true;
  }

  if (HySearchIsHexQuery(trimmedValue)) {
    return HySearchIsFullHash(trimmedValue);
  }

  return trimmedValue.length >= MIN_EXACT_LOOKUP_LEN;
}
