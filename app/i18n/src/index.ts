/**
 * Shared i18n runtime for source-string keyed translations.
 */

import IntlMessageFormat from "intl-messageformat";
import JSON5 from "json5";
import { createSubscriber } from "svelte/reactivity";

/**
 * Dictionary of translated messages keyed by their English source strings.
 */
export interface I18nDictionary {
  readonly [source: string]: string;
}

/**
 * Structured locale catalog stored in KEYVALUEJSON-style documents.
 */
export interface I18nCatalogDocument {
  readonly meta: {
    readonly name: string;
  };
  readonly strings: I18nDictionary;
}

/**
 * Structured locale metadata describing all available language files.
 */
export interface I18nMetaDocument {
  readonly langs: readonly I18nMetaLanguage[];
  readonly blocklist: readonly string[];
}

/**
 * One language entry from the shared locale metadata.
 */
export interface I18nMetaLanguage {
  readonly locale: string;
  readonly name: string;
  readonly src: string;
  readonly dict: Readonly<Record<string, string>>;
}

/**
 * Minimal locale subscription surface exposed to Svelte consumers.
 */
export interface I18nLocaleStore {
  subscribe(run: (locale: string) => void): () => void;
}

/**
 * Primitive formatting argument accepted by `tr()` and `fmt()`.
 */
export type I18nArg = string | number | bigint | boolean | null | undefined;

/**
 * Named ICU formatting arguments accepted by `tr()` and `fmt()`.
 */
export interface I18nNamedArgs {
  readonly [name: string]: I18nArg;
}

interface LoadedDictionary {
  readonly localeTag: string;
  readonly messages: I18nDictionary;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value != null && !Array.isArray(value);
}

function isNamedArgs(value: unknown): value is I18nNamedArgs {
  return isRecord(value);
}

function formatTemplate(source: string, args: readonly I18nArg[]): string {
  let index = 0;

  return source.replaceAll("{}", (): string => {
    const value = args[index];
    index += 1;
    return value == null ? "" : String(value);
  });
}

function normalizeMessageValues(args: readonly I18nArg[] | I18nNamedArgs): Record<string, string | number | boolean> {
  if (Array.isArray(args)) {
    const normalized: Record<string, string | number | boolean> = {};
    const positionalArgs: readonly I18nArg[] = args;

    for (let index = 0; index < positionalArgs.length; index += 1) {
      normalized[String(index)] = normalizeMessageValue(positionalArgs[index]);
    }

    return normalized;
  }

  const normalized: Record<string, string | number | boolean> = {};

  for (const [key, value] of Object.entries(args)) {
    normalized[key] = normalizeMessageValue(value);
  }

  return normalized;
}

function normalizeMessageValue(value: I18nArg): string | number | boolean {
  if (typeof value === "number" || typeof value === "boolean") {
    return value;
  }

  if (typeof value === "bigint") {
    return value.toString();
  }

  return value ?? "";
}

function formatMessage(template: string, args: readonly I18nArg[] | I18nNamedArgs, localeTag: string): string {
  if (Array.isArray(args) && template.includes("{}")) {
    return formatTemplate(template, args);
  }

  const formatter = new IntlMessageFormat(template, localeTag);
  const output = formatter.format(normalizeMessageValues(args));

  if (Array.isArray(output)) {
    return output.join("");
  }

  return String(output);
}

function parseStrings(value: unknown, source: string): I18nDictionary {
  if (!isRecord(value)) {
    throw new Error("Expected strings object in " + source);
  }

  const catalog: Record<string, string> = {};

  for (const [key, entryValue] of Object.entries(value)) {
    if (typeof entryValue !== "string") {
      throw new Error("Expected string translation for key " + JSON.stringify(key) + " in " + source);
    }

    catalog[key] = entryValue;
  }

  return catalog;
}

/**
 * Parses one locale catalog document from JSON5 text.
 */
export function parseCatalogDocument(text: string, source = "<catalog>"): I18nCatalogDocument {
  const parsed: unknown = JSON5.parse(text);

  if (!isRecord(parsed) || !isRecord(parsed["meta"]) || typeof parsed["meta"]["name"] !== "string") {
    throw new Error("Expected locale catalog meta.name in " + source);
  }

  return {
    meta: {
      name: parsed["meta"]["name"],
    },
    strings: parseStrings(parsed["strings"], source),
  };
}

/**
 * Parses one shared locale metadata document from JSON5 text.
 */
export function parseMetaDocument(text: string, source = "<meta>"): I18nMetaDocument {
  const parsed: unknown = JSON5.parse(text);

  if (!isRecord(parsed) || !isRecord(parsed["langs"])) {
    throw new Error("Expected langs object in " + source);
  }

  if (
    !Array.isArray(parsed["blocklist"]) ||
    !parsed["blocklist"].every((entry): boolean => typeof entry === "string")
  ) {
    throw new Error("Expected blocklist string array in " + source);
  }

  const langsObject = parsed["langs"];

  return {
    langs: Object.entries(langsObject).map(([locale, entry], index): I18nMetaLanguage => {
      if (!isRecord(entry) || typeof entry["name"] !== "string" || typeof entry["src"] !== "string") {
        throw new Error("Expected langs[" + String(index) + "] to contain name and src in " + source);
      }

      const dict = entry["dict"];
      if (dict != null && !isRecord(dict)) {
        throw new Error("Expected langs[" + String(index) + "].dict to be an object in " + source);
      }

      const normalizedDict: Record<string, string> = {};
      if (dict != null) {
        for (const [key, value] of Object.entries(dict)) {
          if (typeof value !== "string") {
            throw new Error(
              "Expected langs[" + String(index) + "].dict[" + JSON.stringify(key) + "] to be a string in " + source
            );
          }

          normalizedDict[key] = value;
        }
      }

      return {
        dict: normalizedDict,
        name: entry["name"],
        locale,
        src: entry["src"],
      };
    }),
    blocklist: parsed["blocklist"],
  };
}

const localeSubscribers = new Set<(locale: string) => void>();
const dictionaries = new Map<string, LoadedDictionary>();

let activeLocale = "en";

function syncDocumentLocale(locale: string): void {
  if (typeof document === "undefined") {
    return;
  }

  const localeTag = getDictionary(locale)?.localeTag ?? locale;
  document.documentElement.lang = localeTag;
  document.documentElement.dataset["hyphenLocale"] = locale;
}

const trackLocale = createSubscriber((update: () => void): (() => void) => {
  const subscriber = (): void => {
    update();
  };

  localeSubscribers.add(subscriber);

  return (): void => {
    localeSubscribers.delete(subscriber);
  };
});

function notifyLocaleSubscribers(): void {
  for (const subscriber of localeSubscribers) {
    subscriber(activeLocale);
  }
}

function getDictionary(locale: string): LoadedDictionary | null {
  return dictionaries.get(locale) ?? null;
}

function getActiveLocaleTag(): string {
  return getDictionary(activeLocale)?.localeTag ?? activeLocale;
}

/**
 * Returns the currently active locale code.
 */
export function getLocale(): string {
  trackLocale();
  return activeLocale;
}

/**
 * Returns whether the runtime can serve the requested locale.
 */
export function hasLocale(locale: string): boolean {
  return dictionaries.has(locale) || locale === "en";
}

/**
 * Switches the active locale and notifies locale subscribers.
 */
export function setLocale(locale: string): void {
  activeLocale = locale;
  syncDocumentLocale(locale);
  notifyLocaleSubscribers();
}

/**
 * Loads one locale dictionary into the runtime.
 */
export function loadDictionary(locale: string, messages: I18nDictionary, localeTag = locale): void {
  dictionaries.set(locale, { localeTag, messages: { ...messages } });
  syncDocumentLocale(activeLocale);
  notifyLocaleSubscribers();
}

/**
 * Removes one locale dictionary from the runtime.
 */
export function clearDictionary(locale: string): void {
  dictionaries.delete(locale);
  syncDocumentLocale(activeLocale);
  notifyLocaleSubscribers();
}

/**
 * Sets the active locale during bootstrap without notifying subscribers.
 */
export function primeLocale(locale: string): void {
  activeLocale = locale;
  syncDocumentLocale(locale);
}

/**
 * Resolves one translated string and falls back to the source string when
 * there is no active translation.
 */
export function tr(source: string, args: I18nNamedArgs): string;
export function tr(source: string, ...args: readonly I18nArg[]): string;
export function tr(source: string, ...args: readonly unknown[]): string {
  const dictionary = getDictionary(getLocale());
  const template = dictionary?.messages[source] ?? source;

  if (args.length === 0) {
    return template;
  }

  const formatArgs = args.length === 1 && isNamedArgs(args[0]) ? args[0] : (args as readonly I18nArg[]);
  return formatMessage(template, formatArgs, getActiveLocaleTag());
}

/**
 * Formats one non-translated string template.
 */
export function fmt(source: string, args: I18nNamedArgs): string;
export function fmt(source: string, ...args: readonly I18nArg[]): string;
export function fmt(source: string, ...args: readonly unknown[]): string {
  if (args.length === 0) {
    return source;
  }

  const formatArgs = args.length === 1 && isNamedArgs(args[0]) ? args[0] : (args as readonly I18nArg[]);
  return formatMessage(source, formatArgs, getActiveLocaleTag());
}

/**
 * Locale subscription store used by Svelte consumers.
 */
export const localeStore: I18nLocaleStore = {
  subscribe(run: (locale: string) => void): () => void {
    localeSubscribers.add(run);
    run(activeLocale);

    return (): void => {
      localeSubscribers.delete(run);
    };
  },
};
