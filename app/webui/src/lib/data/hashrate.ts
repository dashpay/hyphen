import type { ChartPoint } from "./chart";

export const TIME_RANGES = ["1h", "1d", "1w", "1m", "1y", "All"] as const;

export type TimeRange = (typeof TIME_RANGES)[number];

const RANGE_SECONDS: Readonly<Record<Exclude<TimeRange, "All">, number>> = {
  "1h": 60 * 60,
  "1d": 24 * 60 * 60,
  "1w": 7 * 24 * 60 * 60,
  "1m": 30 * 24 * 60 * 60,
  "1y": 365 * 24 * 60 * 60,
};

/** Returns the time window in seconds for one range token. */
export function getTimeRangeSeconds(range: TimeRange): number | null {
  if (range === "All") {
    return null;
  }

  return RANGE_SECONDS[range];
}

/** Narrows chart history to one time window using block spacing config. */
export function filterByRange(
  data: readonly ChartPoint[],
  range: TimeRange,
  blockTimeSeconds: number,
  bucketBlocks: number
): ChartPoint[] {
  if (range === "All") {
    return [...data];
  }

  const secondsPerPoint = Math.max(blockTimeSeconds, 1) * Math.max(bucketBlocks, 1);
  const pointCount = Math.max(1, Math.ceil(RANGE_SECONDS[range] / secondsPerPoint));
  return data.slice(-pointCount);
}

/** Expands compact range tokens into the labels shown in the overlay. */
export function formatTimeRangeLabel(range: TimeRange): string {
  return range;
}
