import { browser } from "$app/environment";
import {
  getLocale,
  hasLocale,
  loadDictionary,
  parseCatalogDocument,
  parseMetaDocument,
  primeLocale,
  setLocale,
  type I18nCatalogDocument,
  type I18nMetaDocument,
} from "@repo/hyphen-i18n";

import i18nMetaText from "../../../../../contrib/i18n/meta.json5?raw";

const STORAGE_KEY = "hyphen-webui-locale";
const DEFAULT_LOCALE = "en";
const I18N_META: I18nMetaDocument = parseMetaDocument(i18nMetaText, "contrib/i18n/meta.json5");
const localeCatalogModules = import.meta.glob<string>("../../../../../contrib/i18n/*.json5", {
  eager: true,
  import: "default",
  query: "?raw",
});

export interface WebUiLocaleOption {
  readonly code: string;
  readonly label: string;
}

interface LocaleDefinition {
  readonly code: string;
  readonly locale: string;
  readonly nativeName: string;
  readonly src: string;
}

const localeDefinitions = I18N_META.langs.map(
  (entry): LocaleDefinition => ({
    code: entry.src.replace(/\.json5$/u, ""),
    locale: entry.locale,
    nativeName: entry.name,
    src: entry.src,
  })
);

function getLocaleDefinition(code: string): LocaleDefinition | null {
  return localeDefinitions.find((entry): boolean => entry.code === code) ?? null;
}

function ensureLocaleDictionaries(): void {
  for (const entry of localeDefinitions) {
    if (entry.code === DEFAULT_LOCALE || hasLocale(entry.code)) {
      continue;
    }

    const catalogPath = Object.keys(localeCatalogModules).find((path): boolean => path.endsWith("/" + entry.src));
    const catalogText = catalogPath == null ? undefined : localeCatalogModules[catalogPath];
    if (catalogText == null) {
      continue;
    }

    const catalog: I18nCatalogDocument = parseCatalogDocument(catalogText, "contrib/i18n/" + entry.src);
    loadDictionary(entry.code, catalog.strings, entry.locale);
  }
}

function normalizeLocaleCode(locale: string | null): string {
  ensureLocaleDictionaries();

  if (locale != null && hasLocale(locale)) {
    return locale;
  }

  return DEFAULT_LOCALE;
}

function readBootstrappedLocale(): string | null {
  if (!browser) {
    return null;
  }

  const documentLocale = globalThis.document.documentElement.dataset["hyphenLocale"];
  if (typeof documentLocale === "string" && documentLocale.length > 0) {
    return documentLocale;
  }

  const windowLocale = (globalThis as typeof globalThis & { __HYPHEN_LOCALE__?: unknown }).__HYPHEN_LOCALE__;
  return typeof windowLocale === "string" && windowLocale.length > 0 ? windowLocale : null;
}

ensureLocaleDictionaries();
primeLocale(normalizeLocaleCode(readBootstrappedLocale()));

/**
 * Returns the supported locale options for the shell dropdown.
 */
export function getLocaleOptions(): readonly WebUiLocaleOption[] {
  return localeDefinitions.map(
    (entry): WebUiLocaleOption => ({
      code: entry.code,
      label: entry.code === "en" ? "English" : entry.nativeName,
    })
  );
}

/**
 * Returns the active locale code.
 */
export function getSelectedLocaleCode(): string {
  return getLocale();
}

/**
 * Returns the active locale tag for Intl formatting.
 */
export function getSelectedLocaleTag(): string {
  return getLocaleDefinition(getSelectedLocaleCode())?.locale ?? "en-US";
}

/**
 * Returns the active locale label for shell controls.
 */
export function getSelectedLocaleLabel(): string {
  const locale = getSelectedLocaleCode();
  return getLocaleOptions().find((option): boolean => option.code === locale)?.label ?? "English";
}

/**
 * Persists the selected locale across route changes.
 */
export function setSelectedLocaleCode(locale: string): void {
  const nextLocale = normalizeLocaleCode(locale);
  setLocale(nextLocale);

  if (browser) {
    globalThis.localStorage.setItem(STORAGE_KEY, nextLocale);
  }
}

/**
 * Restores the selected locale from storage when possible.
 */
export function hydrateSelectedLocale(): void {
  if (!browser) {
    setLocale(DEFAULT_LOCALE);
    return;
  }

  const storageLocale = globalThis.localStorage.getItem(STORAGE_KEY);
  const nextLocale = normalizeLocaleCode(storageLocale ?? readBootstrappedLocale());

  if (storageLocale == null) {
    globalThis.localStorage.setItem(STORAGE_KEY, nextLocale);
  }

  if (getLocale() !== nextLocale) {
    setLocale(nextLocale);
  }
}
