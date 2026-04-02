import {
  VIEW_BREAKPOINTS,
  getViewBreakpointName,
  getViewBreakpointWidth,
  type ViewBreakpointName,
} from "@repo/hyphen-views/breakpoints";
import {
  formatSelectedAmount as formatConfiguredAmount,
  getSelectedDenomination,
  getSelectedDenominationKey,
  getSelectedDenominationLabel,
  hydrateSelectedDenominationKey,
  setSelectedDenominationKey,
} from "./config/selection.svelte";
import {
  getLocaleOptions,
  getSelectedLocaleCode,
  getSelectedLocaleLabel,
  getSelectedLocaleTag,
  hydrateSelectedLocale,
  setSelectedLocaleCode,
  type WebUiLocaleOption,
} from "./config/locale.svelte";
import type { WebUiConfig, WebUiDenomination } from "./config/schema";
import {
  formatBlockTime as formatCompactBlockTime,
  formatDate as formatCompactDate,
  formatBytes as formatCompactBytes,
  formatDifficulty as formatCompactDifficulty,
  formatFullDate as formatDetailedDate,
  formatHashRate as formatCompactHashRate,
  formatLocaleNumber as formatLocalizedNumber,
  formatRelativeDate as formatRelativeDate,
} from "./data/format";

export const HyUtilWebUiName = "hyphen";

export const HyUtilBreakpoints = VIEW_BREAKPOINTS;

export type HyUtilBreakpointName = ViewBreakpointName;

/** Formats a Unix timestamp or ISO date as a compact 24-hour time string. */
export function HyUtilFormatBlockTime(timestamp: number | string): string {
  return formatCompactBlockTime(timestamp);
}

/** Formats a Unix timestamp or ISO date as a date plus 24-hour time. */
export function HyUtilFormatFullDate(timestamp: number | string): string {
  return formatDetailedDate(timestamp);
}

/** Formats one Unix timestamp or ISO date relative to now. */
export function HyUtilFormatRelativeDate(timestamp: number | string): string {
  return formatRelativeDate(timestamp);
}

/** Formats a Unix timestamp or ISO date as a localized date string. */
export function HyUtilFormatDate(timestamp: number | string): string {
  return formatCompactDate(timestamp);
}

/** Formats bytes with the compact explorer units. */
export function HyUtilFormatBytes(bytes: number): string {
  return formatCompactBytes(bytes);
}

/** Formats hashrate with the first matching unit and /s suffix. */
export function HyUtilFormatHashRate(hashRate: number): string {
  return formatCompactHashRate(hashRate);
}

/** Formats difficulty with compact SI suffixes for explorer cards. */
export function HyUtilFormatDifficulty(difficulty: number): string {
  return formatCompactDifficulty(difficulty);
}

/** Formats one number with the active locale. */
export function HyUtilFormatNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return formatLocalizedNumber(value, options);
}

/** Returns the pixel width for one named Hyphen breakpoint. */
export function HyUtilGetBreakpointWidth(name: HyUtilBreakpointName): number {
  return getViewBreakpointWidth(name);
}

/** Returns the named breakpoint that contains one measured width. */
export function HyUtilGetBreakpointName(width: number): HyUtilBreakpointName {
  return getViewBreakpointName(width);
}

/** Returns the active denomination key for the current config. */
export function HyUtilGetSelectedDenominationKey(config: WebUiConfig | null | undefined): string {
  return getSelectedDenominationKey(config);
}

/** Returns the active denomination label for shell controls. */
export function HyUtilGetSelectedDenominationLabel(config: WebUiConfig | null | undefined): string {
  return getSelectedDenominationLabel(config);
}

/** Returns the active denomination object for app formatting. */
export function HyUtilGetSelectedDenomination(config: WebUiConfig | null | undefined): WebUiDenomination {
  return getSelectedDenomination(config);
}

/** Formats one amount with the active denomination and ticker. */
export function HyUtilFormatSelectedAmount(
  value: number,
  config: WebUiConfig | null | undefined,
  maxDecimals?: number
): string {
  return formatConfiguredAmount(value, config, maxDecimals);
}

/** Formats one table amount with stable precision. */
export function HyUtilFormatTableAmount(
  value: number,
  config: WebUiConfig | null | undefined,
  fullDecimals?: number
): string {
  return formatConfiguredAmount(value, config, fullDecimals);
}

/** Persists the selected denomination across route changes. */
export function HyUtilSetSelectedDenominationKey(key: string): void {
  setSelectedDenominationKey(key);
}

/** Restores the selected denomination when storage is available. */
export function HyUtilHydrateSelectedDenominationKey(config: WebUiConfig | null | undefined): void {
  hydrateSelectedDenominationKey(config);
}

/** Returns the supported locale options for shell controls. */
export function HyUtilGetLocaleOptions(): readonly WebUiLocaleOption[] {
  return getLocaleOptions();
}

/** Returns the active locale code. */
export function HyUtilGetSelectedLocaleCode(): string {
  return getSelectedLocaleCode();
}

/** Returns the active locale label for shell controls. */
export function HyUtilGetSelectedLocaleLabel(): string {
  return getSelectedLocaleLabel();
}

/** Returns the active locale tag for Intl formatting. */
export function HyUtilGetSelectedLocaleTag(): string {
  return getSelectedLocaleTag();
}

/** Persists the selected locale across route changes. */
export function HyUtilSetSelectedLocaleCode(locale: string): void {
  setSelectedLocaleCode(locale);
}

/** Restores the selected locale when storage is available. */
export function HyUtilHydrateSelectedLocale(): void {
  hydrateSelectedLocale();
}

/** Reads one unknown failure into stable route and panel copy. */
export function HyUtilReadErrorMessage(caught: unknown, fallback = "Unknown error"): string {
  if (caught instanceof Error && caught.message.length > 0) {
    return caught.message;
  }

  if (typeof caught === "string" && caught.length > 0) {
    return caught;
  }

  if (caught != null && typeof caught === "object") {
    const errorRecord = caught as Record<string, unknown>;
    const error = errorRecord["error"];

    if (typeof error === "string" && error.length > 0) {
      return error;
    }

    if (error != null && typeof error === "object") {
      const errorObject = error as Record<string, unknown>;
      if (typeof errorObject["message"] === "string" && errorObject["message"].length > 0) {
        return errorObject["message"];
      }
    }

    if (typeof errorRecord["message"] === "string" && errorRecord["message"].length > 0) {
      return errorRecord["message"];
    }
  }

  return fallback;
}

/** Reads one unknown failure into an HTTP status when one is available. */
export function HyUtilReadErrorStatus(caught: unknown): number | null {
  if (caught != null && typeof caught === "object") {
    const errorRecord = caught as Record<string, unknown>;

    if (typeof errorRecord["status"] === "number" && Number.isInteger(errorRecord["status"])) {
      return errorRecord["status"];
    }
  }

  return null;
}
