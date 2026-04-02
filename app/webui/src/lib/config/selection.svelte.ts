import { browser } from "$app/environment";

import { formatDenominatedAmount, getCurrencyConfig, getDefaultDenomination, getDenomination } from "./format";
import type { WebUiConfig, WebUiDenomination } from "./schema";

const STORAGE_KEY = "hyphen-webui-denomination";

let selectedDenominationKey = $state<string | null>(null);

/** Returns the active denomination key or the configured default. */
export function getSelectedDenominationKey(config: WebUiConfig | null | undefined): string {
  return selectedDenominationKey ?? getDefaultDenomination(config).key;
}

/** Returns the active denomination label for header controls. */
export function getSelectedDenominationLabel(config: WebUiConfig | null | undefined): string {
  return getDenomination(config, getSelectedDenominationKey(config)).label;
}

/** Returns the active denomination object for amount formatting. */
export function getSelectedDenomination(config: WebUiConfig | null | undefined): WebUiDenomination {
  return getDenomination(config, getSelectedDenominationKey(config));
}

/** Formats one amount with the active denomination and ticker. */
export function formatSelectedAmount(
  value: number,
  config: WebUiConfig | null | undefined,
  maxDecimals?: number
): string {
  const currency = getCurrencyConfig(config);
  return formatDenominatedAmount(value, getSelectedDenomination(config), currency.ticker, maxDecimals);
}

/** Persists the selected denomination so route changes keep it. */
export function setSelectedDenominationKey(key: string): void {
  selectedDenominationKey = key;

  if (browser) {
    globalThis.localStorage.setItem(STORAGE_KEY, key);
  }
}

/** Restores the selected denomination from storage when possible. */
export function hydrateSelectedDenominationKey(config: WebUiConfig | null | undefined): void {
  if (!browser) {
    selectedDenominationKey = getDefaultDenomination(config).key;
    return;
  }

  const storedValue = globalThis.localStorage.getItem(STORAGE_KEY);
  selectedDenominationKey =
    storedValue && storedValue.length > 0
      ? getDenomination(config, storedValue).key
      : getDefaultDenomination(config).key;
}
