import { getSelectedLocaleTag } from "../config/locale.svelte";

function toDate(timestamp: number | string): Date {
  return typeof timestamp === "number" ? new Date(timestamp * 1_000) : new Date(timestamp);
}

function toRelativeUnit(diffSeconds: number): readonly [Intl.RelativeTimeFormatUnit, number] {
  const absSeconds = Math.abs(diffSeconds);

  if (absSeconds < 60) {
    return ["second", diffSeconds];
  }

  const diffMinutes = Math.round(diffSeconds / 60);
  if (Math.abs(diffMinutes) < 60) {
    return ["minute", diffMinutes];
  }

  const diffHours = Math.round(diffMinutes / 60);
  if (Math.abs(diffHours) < 24) {
    return ["hour", diffHours];
  }

  const diffDays = Math.round(diffHours / 24);
  if (Math.abs(diffDays) < 30) {
    return ["day", diffDays];
  }

  const diffMonths = Math.round(diffDays / 30);
  if (Math.abs(diffMonths) < 12) {
    return ["month", diffMonths];
  }

  return ["year", Math.round(diffDays / 365)];
}

/**
 * Formats one number with the active locale.
 */
export function formatLocaleNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(getSelectedLocaleTag(), options).format(value);
}

/**
 * Formats a Unix timestamp or ISO date as a compact 24-hour time string.
 */
export function formatBlockTime(timestamp: number | string): string {
  const date = toDate(timestamp);
  return date.toLocaleTimeString(getSelectedLocaleTag(), {
    hour: "2-digit",
    hour12: false,
    minute: "2-digit",
    second: "2-digit",
  });
}

/**
 * Formats a Unix timestamp or ISO date as a localized date string.
 */
export function formatDate(timestamp: number | string): string {
  const date = toDate(timestamp);
  return date.toLocaleDateString(getSelectedLocaleTag(), { day: "2-digit", month: "short", year: "numeric" });
}

/**
 * Formats a Unix timestamp or ISO date as a date plus 24-hour time.
 */
export function formatFullDate(timestamp: number | string): string {
  const date = toDate(timestamp);
  return (
    date.toLocaleDateString(getSelectedLocaleTag(), { day: "2-digit", month: "short", year: "numeric" }) +
    ", " +
    date.toLocaleTimeString(getSelectedLocaleTag(), {
      hour: "2-digit",
      hour12: false,
      minute: "2-digit",
      second: "2-digit",
    })
  );
}

/**
 * Formats one timestamp relative to now using the active locale.
 */
export function formatRelativeDate(timestamp: number | string): string {
  const date = toDate(timestamp);
  const diffSeconds = Math.round((date.getTime() - Date.now()) / 1_000);
  const [unit, value] = toRelativeUnit(diffSeconds);
  return new Intl.RelativeTimeFormat(getSelectedLocaleTag(), { numeric: "auto" }).format(value, unit);
}

/**
 * Formats bytes with the compact units used in explorer panels.
 */
export function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(bytes / 1_000_000_000) + " GB"
    );
  }

  if (bytes >= 1_000_000) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(bytes / 1_000_000) + " MB"
    );
  }

  return (
    new Intl.NumberFormat(getSelectedLocaleTag(), {
      maximumFractionDigits: 2,
      minimumFractionDigits: 2,
    }).format(bytes / 1_000) + " KB"
  );
}

/**
 * Formats hashrate with the first matching SI unit and /s suffix.
 */
export function formatHashRate(hashRate: number): string {
  if (hashRate >= 1e15) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(hashRate / 1e15) + " PH/s"
    );
  }

  if (hashRate >= 1e12) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(hashRate / 1e12) + " TH/s"
    );
  }

  if (hashRate >= 1e9) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(hashRate / 1e9) + " GH/s"
    );
  }

  if (hashRate >= 1e6) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(hashRate / 1e6) + " MH/s"
    );
  }

  if (hashRate >= 1e3) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(hashRate / 1e3) + " kH/s"
    );
  }

  return new Intl.NumberFormat(getSelectedLocaleTag(), { maximumFractionDigits: 0 }).format(hashRate) + " H/s";
}

/**
 * Formats difficulty with compact SI suffixes for metrics cards.
 */
export function formatDifficulty(difficulty: number): string {
  if (difficulty >= 1e12) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(difficulty / 1e12) + "T"
    );
  }

  if (difficulty >= 1e9) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(difficulty / 1e9) + "G"
    );
  }

  if (difficulty >= 1e6) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(difficulty / 1e6) + "M"
    );
  }

  if (difficulty >= 1e3) {
    return (
      new Intl.NumberFormat(getSelectedLocaleTag(), {
        maximumFractionDigits: 2,
        minimumFractionDigits: 2,
      }).format(difficulty / 1e3) + "K"
    );
  }

  return new Intl.NumberFormat(getSelectedLocaleTag(), {
    maximumFractionDigits: 2,
    minimumFractionDigits: 2,
  }).format(difficulty);
}
