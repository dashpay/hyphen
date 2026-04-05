import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

import JSON5 from "json5";

/**
 * Translation catalog keyed by English source strings.
 */
export interface TranslationCatalog {
  readonly [source: string]: string;
}

/**
 * Structured locale catalog stored on disk.
 */
export interface LocaleCatalogDocument {
  readonly meta: {
    readonly name: string;
  };
  readonly strings: TranslationCatalog;
}

/**
 * Shared i18n metadata document stored on disk.
 */
export interface LocaleMetaDocument {
  readonly langs: readonly {
    readonly dict: Readonly<Record<string, string>>;
    readonly locale: string;
    readonly name: string;
    readonly src: string;
  }[];
  readonly blocklist: readonly string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value != null && !Array.isArray(value);
}

/**
 * Returns one translation catalog with its keys sorted deterministically.
 */
export function sortCatalog(catalog: TranslationCatalog): TranslationCatalog {
  const sortedEntries = Object.entries(catalog).sort(([left], [right]) => left.localeCompare(right));
  return Object.fromEntries(sortedEntries);
}

function readCatalogEntries(path: string, value: unknown): TranslationCatalog {
  if (!isRecord(value)) {
    throw new Error("Expected translation catalog object in " + path);
  }

  const catalog: Record<string, string> = {};

  for (const [key, entryValue] of Object.entries(value)) {
    if (typeof entryValue !== "string") {
      throw new Error("Expected string translation for key " + JSON.stringify(key) + " in " + path);
    }

    catalog[key] = entryValue;
  }

  return catalog;
}

/**
 * Reads one locale catalog document from disk and validates its structure.
 */
export async function readCatalogDocument(path: string): Promise<LocaleCatalogDocument> {
  const raw = await readFile(path, "utf8");
  const parsed: unknown = JSON5.parse(raw);

  if (!isRecord(parsed)) {
    throw new Error("Expected locale catalog document in " + path);
  }

  const meta = parsed["meta"];
  const strings = parsed["strings"];

  if (!isRecord(meta) || typeof meta["name"] !== "string" || meta["name"].length === 0) {
    throw new Error("Expected locale catalog meta.name in " + path);
  }

  return {
    meta: {
      name: meta["name"],
    },
    strings: readCatalogEntries(path, strings),
  };
}

/**
 * Reads one translation catalog from disk.
 */
export async function readCatalog(path: string): Promise<TranslationCatalog> {
  const document = await readCatalogDocument(path);
  return document.strings;
}

/**
 * Reads the shared locale metadata document from disk.
 */
export async function readMetaDocument(path: string): Promise<LocaleMetaDocument> {
  const raw = await readFile(path, "utf8");
  const parsed: unknown = JSON5.parse(raw);

  if (!isRecord(parsed)) {
    throw new Error("Expected locale metadata document in " + path);
  }

  const langs = parsed["langs"];
  const blocklist = parsed["blocklist"];

  if (!isRecord(langs)) {
    throw new Error("Expected langs object in " + path);
  }

  if (!Array.isArray(blocklist) || !blocklist.every((entry): boolean => typeof entry === "string")) {
    throw new Error("Expected blocklist string array in " + path);
  }

  return {
    langs: Object.entries(langs).map(([locale, entry], index) => {
      if (!isRecord(entry) || typeof entry["name"] !== "string" || typeof entry["src"] !== "string") {
        throw new Error("Expected langs[" + String(index) + "] to contain name and src in " + path);
      }

      const dict = entry["dict"];
      if (dict != null && !isRecord(dict)) {
        throw new Error("Expected langs[" + String(index) + "].dict to be an object in " + path);
      }

      const normalizedDict: Record<string, string> = {};
      if (dict != null) {
        for (const [key, value] of Object.entries(dict)) {
          if (typeof value !== "string") {
            throw new Error(
              "Expected langs[" + String(index) + "].dict[" + JSON.stringify(key) + "] to be a string in " + path
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
    blocklist,
  };
}

export function serializeJson5(value: unknown): string {
  return JSON5.stringify(value, null, 2) + "\n";
}

/**
 * Writes one locale catalog document to disk using deterministic key ordering.
 */
export async function writeCatalogDocument(path: string, document: LocaleCatalogDocument): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const json = serializeJson5({
    meta: {
      name: document.meta.name,
    },
    strings: sortCatalog(document.strings),
  });
  await writeFile(path, json, "utf8");
}

/**
 * Writes one translation catalog to disk using deterministic key ordering.
 */
export async function writeCatalog(path: string, catalog: TranslationCatalog, name: string): Promise<void> {
  await writeCatalogDocument(path, {
    meta: {
      name,
    },
    strings: catalog,
  });
}
