import type { I18nDictionary, I18nMetaLanguage } from "@repo/hyphen-i18n";
import type { WebUiConfig } from "./schema";

export interface ValidatorTermOverrideSelection {
  readonly pluralKey: string;
  readonly singularKey: string;
}

export function readValidatorTermOverrideFromConfig(config: WebUiConfig): ValidatorTermOverrideSelection | null {
  const singularKey = config.features.validators.termSingular;
  const pluralKey = config.features.validators.termPlural;
  return singularKey == null || pluralKey == null ? null : { pluralKey, singularKey };
}

const VALIDATOR_COUNT_KEY = "{count, plural, one {# validator} other {# validators}}";
const VALIDATOR_ACTIVE_COUNT_KEY =
  "{totalCount, plural, one {# validator} other {# validators}} ({enabledCount} active)";

interface TermReplacement {
  readonly search: string;
  readonly replace: string;
}

function upperFirst(value: string): string {
  if (value.length === 0) {
    return value;
  }

  return value.charAt(0).toLocaleUpperCase() + value.slice(1);
}

function readOverrideValue(dict: Readonly<Record<string, string>>, key: string): string | null {
  const value = dict[key];
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

function applyTermReplacements(value: string, replacements: readonly TermReplacement[]): string {
  return replacements.reduce((nextValue: string, replacement: TermReplacement): string => {
    if (replacement.search.length === 0 || replacement.search === replacement.replace) {
      return nextValue;
    }

    return nextValue.replaceAll(replacement.search, replacement.replace);
  }, value);
}

export function applyValidatorTermOverridesForSelection(
  strings: I18nDictionary,
  language: I18nMetaLanguage,
  selection: ValidatorTermOverrideSelection | null
): I18nDictionary {
  if (selection == null) {
    return strings;
  }

  const singular = readOverrideValue(language.dict, selection.singularKey);
  const plural = readOverrideValue(language.dict, selection.pluralKey);
  if (singular == null || plural == null) {
    return strings;
  }

  const nextStrings: Record<string, string> = {
    ...strings,
    validator: singular,
    Validator: upperFirst(singular),
    Validators: upperFirst(plural),
  };

  const votedLabel = readOverrideValue(language.dict, selection.pluralKey + "_voted");
  if (votedLabel != null) {
    nextStrings["Validators Voted"] = votedLabel;
  }

  const countLabel = readOverrideValue(language.dict, selection.pluralKey + "_count");
  if (countLabel != null) {
    nextStrings[VALIDATOR_COUNT_KEY] = countLabel;
  }

  const activeCountLabel = readOverrideValue(language.dict, selection.pluralKey + "_active_count");
  if (activeCountLabel != null) {
    nextStrings[VALIDATOR_ACTIVE_COUNT_KEY] = activeCountLabel;
  }

  const replacements: readonly TermReplacement[] = [
    { search: strings["Validators"] ?? "Validators", replace: upperFirst(plural) },
    { search: strings["validator"] ?? "validator", replace: singular },
    { search: strings["Validator"] ?? "Validator", replace: upperFirst(singular) },
  ];

  for (const [key, value] of Object.entries(nextStrings)) {
    nextStrings[key] = applyTermReplacements(value, replacements);
  }

  return nextStrings;
}

export function applyValidatorTermOverrides(
  strings: I18nDictionary,
  language: I18nMetaLanguage,
  selection: ValidatorTermOverrideSelection | null
): I18nDictionary {
  return applyValidatorTermOverridesForSelection(strings, language, selection);
}
