import { DEFAULT_WEBUI_CONFIG, type WebUiConfig, type WebUiCurrencyConfig, type WebUiDenomination } from "./schema";
import { getSelectedLocaleTag } from "./locale.svelte";

const FALLBACK_DENOMINATION: WebUiDenomination = DEFAULT_WEBUI_CONFIG.currency.denominations[0] ?? {
  key: "dash",
  label: "Dash",
  factor: 100_000_000,
  decimals: 8,
};

function withThousands(value: string): string {
  const [integerPart, fractionalPart] = value.split(".");
  const normalizedInteger = new Intl.NumberFormat(getSelectedLocaleTag(), { maximumFractionDigits: 0 }).format(
    Number(integerPart)
  );
  return fractionalPart != null ? normalizedInteger + "." + fractionalPart : normalizedInteger;
}

/** Returns currency metadata from config or the built-in defaults. */
export function getCurrencyConfig(config: WebUiConfig | null | undefined): WebUiCurrencyConfig {
  return config?.currency ?? DEFAULT_WEBUI_CONFIG.currency;
}

/** Returns the default denomination for the active currency config. */
export function getDefaultDenomination(config: WebUiConfig | null | undefined): WebUiDenomination {
  const currency = getCurrencyConfig(config);
  return getDenomination(config, currency.defaultDenomination);
}

/** Resolves one denomination key against the active currency config. */
export function getDenomination(config: WebUiConfig | null | undefined, key: string): WebUiDenomination {
  const currency = getCurrencyConfig(config);
  return (
    currency.denominations.find((denomination: WebUiDenomination): boolean => denomination.key === key) ??
    currency.denominations[0] ??
    FALLBACK_DENOMINATION
  );
}

/** Formats duffs using one denomination and an optional decimal cap. */
export function formatDenominatedAmount(
  value: number,
  denomination: WebUiDenomination,
  ticker: string,
  maxDecimals?: number
): string {
  const safeFactor = denomination.factor > 0 ? denomination.factor : FALLBACK_DENOMINATION.factor;
  const safeDecimals =
    Number.isInteger(denomination.decimals) && denomination.decimals >= 0 && denomination.decimals <= 100
      ? denomination.decimals
      : FALLBACK_DENOMINATION.decimals;
  const displayDecimals =
    maxDecimals != null && Number.isInteger(maxDecimals) && maxDecimals >= 0
      ? Math.min(maxDecimals, safeDecimals)
      : safeDecimals;
  const normalizedValue = value / safeFactor;
  const unitLabel = denomination.label.length > 0 ? denomination.label : ticker;
  return withThousands(normalizedValue.toFixed(displayDecimals)) + " " + unitLabel;
}

/** Splits one formatted amount into number and unit segments. */
export function splitDenominatedAmount(value: string): { readonly amount: string; readonly unit: string | null } {
  const trimmedValue = value.trim();
  const separatorIndex = trimmedValue.lastIndexOf(" ");

  if (separatorIndex <= 0 || separatorIndex >= trimmedValue.length - 1) {
    return {
      amount: trimmedValue,
      unit: null,
    };
  }

  return {
    amount: trimmedValue.slice(0, separatorIndex),
    unit: trimmedValue.slice(separatorIndex + 1),
  };
}

/** Formats duffs using a denomination key from the active config. */
export function formatAmount(value: number, config: WebUiConfig | null | undefined, denominationKey?: string): string {
  const currency = getCurrencyConfig(config);
  if (denominationKey == null) {
    return formatDenominatedAmount(value, getDefaultDenomination(config), currency.ticker);
  }

  return formatDenominatedAmount(value, getDenomination(config, denominationKey), currency.ticker);
}
