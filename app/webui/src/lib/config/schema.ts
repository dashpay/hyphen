export interface WebUiDenomination {
  readonly key: string;
  readonly label: string;
  readonly factor: number;
  readonly decimals: number;
}

export interface WebUiCurrencyConfig {
  readonly name: string;
  readonly ticker: string;
  readonly defaultDenomination: string;
  readonly denominations: readonly WebUiDenomination[];
}

export interface WebUiBalancesConfig {
  readonly enableConcentration: boolean;
}

export interface WebUiChainlockConfig {
  readonly enabled: boolean;
}

export interface WebUiInstantSendConfig {
  readonly enabled: boolean;
}

export interface WebUiGovernanceConfig {
  readonly enabled: boolean;
  readonly superblockInterval: number;
}

export interface WebUiValidatorsConfig {
  readonly enabled: boolean;
  readonly termPlural: string | null;
  readonly termSingular: string | null;
}

export interface WebUiFeaturesConfig {
  readonly chainlock: WebUiChainlockConfig;
  readonly instantsend: WebUiInstantSendConfig;
  readonly governance: WebUiGovernanceConfig;
  readonly validators: WebUiValidatorsConfig;
}

export interface WebUiNetworkConfig {
  readonly blockTimeSeconds: number;
  readonly creditPoolStartHeight: number;
}

export interface WebUiBlockTagRule {
  readonly label: string;
  readonly tone: string;
  readonly block_hash: string | null;
  readonly height: number | null;
  readonly has_coinbase: boolean | null;
  readonly best_cl_height_present: boolean | null;
}

export interface WebUiConfig {
  readonly balances: WebUiBalancesConfig;
  readonly currency: WebUiCurrencyConfig;
  readonly features: WebUiFeaturesConfig;
  readonly network: WebUiNetworkConfig;
  readonly blockTags: readonly WebUiBlockTagRule[];
}

export const DEFAULT_DENOMINATIONS: readonly WebUiDenomination[] = [
  {
    key: "dash",
    label: "DASH",
    factor: 100_000_000,
    decimals: 8,
  },
  {
    key: "mdash",
    label: "mDASH",
    factor: 100_000,
    decimals: 5,
  },
  {
    key: "udash",
    label: "µDASH",
    factor: 100,
    decimals: 2,
  },
  {
    key: "duff",
    label: "duffs",
    factor: 1,
    decimals: 0,
  },
] as const;

export const DEFAULT_WEBUI_CONFIG: WebUiConfig = {
  balances: {
    enableConcentration: true,
  },
  currency: {
    name: "Dash",
    ticker: "DASH",
    defaultDenomination: "dash",
    denominations: DEFAULT_DENOMINATIONS,
  },
  features: {
    chainlock: {
      enabled: true,
    },
    instantsend: {
      enabled: true,
    },
    governance: {
      enabled: true,
      superblockInterval: 16616,
    },
    validators: {
      enabled: true,
      termPlural: null,
      termSingular: null,
    },
  },
  network: {
    blockTimeSeconds: 150,
    creditPoolStartHeight: 0,
  },
  blockTags: [],
};

const FALLBACK_DENOMINATION: WebUiDenomination = DEFAULT_DENOMINATIONS[0] ?? {
  key: "dash",
  label: "DASH",
  factor: 100_000_000,
  decimals: 8,
};

interface PartialWebUiDenomination {
  readonly key?: unknown;
  readonly label?: unknown;
  readonly factor?: unknown;
  readonly decimals?: unknown;
}

interface PartialWebUiCurrencyConfig {
  readonly name?: unknown;
  readonly ticker?: unknown;
  readonly default_denomination?: unknown;
  readonly denominations?: unknown;
}

interface PartialWebUiBalancesConfig {
  readonly enable_concentration?: unknown;
}

interface PartialWebUiFeatureToggle {
  readonly enabled?: unknown;
  readonly term_plural?: unknown;
  readonly term_singular?: unknown;
}

interface PartialWebUiGovernanceConfig extends PartialWebUiFeatureToggle {
  readonly superblock_interval?: unknown;
}

interface PartialWebUiFeaturesConfig {
  readonly chainlock?: unknown;
  readonly instantsend?: unknown;
  readonly governance?: unknown;
  readonly validators?: unknown;
}

interface PartialWebUiConfig {
  readonly balances?: unknown;
  readonly currency?: unknown;
  readonly features?: unknown;
  readonly network?: unknown;
  readonly block_tags?: unknown;
}

interface PartialWebUiNetworkConfig {
  readonly block_time_seconds?: unknown;
  readonly credit_pool_start_height?: unknown;
}

interface PartialWebUiBlockTagRule {
  readonly label?: unknown;
  readonly tone?: unknown;
  readonly block_hash?: unknown;
  readonly height?: unknown;
  readonly has_coinbase?: unknown;
  readonly best_cl_height_present?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function readString(value: unknown, fallback: string): string {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

function readPositiveNumber(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}

function readNonNegativeInteger(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : fallback;
}

function readNullableString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function readNullableBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function readNullableNonNegativeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : null;
}

function readBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function normalizeDenomination(value: unknown, fallback: WebUiDenomination): WebUiDenomination {
  if (!isRecord(value)) {
    return fallback;
  }

  const record = value as PartialWebUiDenomination;

  return {
    key: readString(record.key, fallback.key),
    label: readString(record.label, fallback.label),
    factor: readPositiveNumber(record.factor, fallback.factor),
    decimals: readNonNegativeInteger(record.decimals, fallback.decimals),
  };
}

function normalizeDenominations(value: unknown): readonly WebUiDenomination[] {
  if (!Array.isArray(value)) {
    return DEFAULT_DENOMINATIONS;
  }

  const normalized = value
    .slice(0, DEFAULT_DENOMINATIONS.length || value.length)
    .map((entry: unknown, index: number): WebUiDenomination => {
      const fallback = DEFAULT_DENOMINATIONS[index] ?? FALLBACK_DENOMINATION;
      return normalizeDenomination(entry, fallback);
    });

  return normalized.length > 0 ? normalized : DEFAULT_DENOMINATIONS;
}

function hasDenomination(denominations: readonly WebUiDenomination[], key: string): boolean {
  return denominations.some((denomination: WebUiDenomination): boolean => denomination.key === key);
}

function normalizeBlockTagRule(value: unknown): WebUiBlockTagRule | null {
  if (!isRecord(value)) {
    return null;
  }

  const record = value as PartialWebUiBlockTagRule;
  const label = readNullableString(record.label);

  if (label == null) {
    return null;
  }

  return {
    label,
    tone: readString(record.tone, "neutral"),
    block_hash: readNullableString(record.block_hash),
    height: readNullableNonNegativeInteger(record.height),
    has_coinbase: readNullableBoolean(record.has_coinbase),
    best_cl_height_present: readNullableBoolean(record.best_cl_height_present),
  };
}

function normalizeBlockTags(value: unknown): readonly WebUiBlockTagRule[] {
  if (!Array.isArray(value)) {
    return [];
  }

  return value
    .map((entry: unknown): WebUiBlockTagRule | null => normalizeBlockTagRule(entry))
    .filter((entry: WebUiBlockTagRule | null): entry is WebUiBlockTagRule => entry != null);
}

export function normalizeWebUiConfig(value: unknown): WebUiConfig {
  if (!isRecord(value)) {
    return DEFAULT_WEBUI_CONFIG;
  }

  const record = value as PartialWebUiConfig;
  const balancesRecord = isRecord(record.balances) ? (record.balances as PartialWebUiBalancesConfig) : {};
  const currencyRecord = isRecord(record.currency) ? (record.currency as PartialWebUiCurrencyConfig) : {};
  const featuresRecord = isRecord(record.features) ? (record.features as PartialWebUiFeaturesConfig) : {};
  const chainlockRecord = isRecord(featuresRecord.chainlock)
    ? (featuresRecord.chainlock as PartialWebUiFeatureToggle)
    : {};
  const instantsendRecord = isRecord(featuresRecord.instantsend)
    ? (featuresRecord.instantsend as PartialWebUiFeatureToggle)
    : {};
  const governanceRecord = isRecord(featuresRecord.governance)
    ? (featuresRecord.governance as PartialWebUiGovernanceConfig)
    : {};
  const validatorsRecord = isRecord(featuresRecord.validators)
    ? (featuresRecord.validators as PartialWebUiFeatureToggle)
    : {};
  const networkRecord = isRecord(record.network) ? (record.network as PartialWebUiNetworkConfig) : {};
  const denominations = normalizeDenominations(currencyRecord.denominations);
  const fallbackDenomination = denominations[0]?.key ?? DEFAULT_WEBUI_CONFIG.currency.defaultDenomination;
  const requestedDefaultDenomination = readString(currencyRecord.default_denomination, fallbackDenomination);
  const defaultDenomination = hasDenomination(denominations, requestedDefaultDenomination)
    ? requestedDefaultDenomination
    : fallbackDenomination;

  return {
    balances: {
      enableConcentration:
        typeof balancesRecord.enable_concentration === "boolean"
          ? balancesRecord.enable_concentration
          : DEFAULT_WEBUI_CONFIG.balances.enableConcentration,
    },
    currency: {
      name: readString(currencyRecord.name, DEFAULT_WEBUI_CONFIG.currency.name),
      ticker: readString(currencyRecord.ticker, DEFAULT_WEBUI_CONFIG.currency.ticker),
      defaultDenomination,
      denominations,
    },
    features: {
      chainlock: {
        enabled: readBoolean(chainlockRecord.enabled, DEFAULT_WEBUI_CONFIG.features.chainlock.enabled),
      },
      instantsend: {
        enabled: readBoolean(instantsendRecord.enabled, DEFAULT_WEBUI_CONFIG.features.instantsend.enabled),
      },
      governance: {
        enabled: readBoolean(governanceRecord.enabled, DEFAULT_WEBUI_CONFIG.features.governance.enabled),
        superblockInterval: readPositiveNumber(
          governanceRecord.superblock_interval,
          DEFAULT_WEBUI_CONFIG.features.governance.superblockInterval
        ),
      },
      validators: {
        enabled: readBoolean(validatorsRecord.enabled, DEFAULT_WEBUI_CONFIG.features.validators.enabled),
        termPlural: readNullableString(validatorsRecord.term_plural),
        termSingular: readNullableString(validatorsRecord.term_singular),
      },
    },
    network: {
      blockTimeSeconds: readPositiveNumber(
        networkRecord.block_time_seconds,
        DEFAULT_WEBUI_CONFIG.network.blockTimeSeconds
      ),
      creditPoolStartHeight: readNonNegativeInteger(
        networkRecord.credit_pool_start_height,
        DEFAULT_WEBUI_CONFIG.network.creditPoolStartHeight
      ),
    },
    blockTags: normalizeBlockTags(record.block_tags),
  };
}
