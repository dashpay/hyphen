import { resolve } from "node:path";

/**
 * Resolved repository paths used by the i18n CLI commands.
 */
export interface RepoPaths {
  readonly metaPath: string;
  readonly localeDir: string;
  readonly repoRoot: string;
  readonly sourceCatalogPath: string;
}

/**
 * Resolves the repository paths used by the i18n CLI commands.
 */
const LOCALE_CATALOG_NAME = /^[a-z]{2}\.json5$/;
export function getRepoPaths(): RepoPaths {
  const repoRoot = resolve(import.meta.dirname, "../../../../../");

  return {
    metaPath: resolve(repoRoot, "contrib/i18n/meta.json5"),
    localeDir: resolve(repoRoot, "contrib/i18n"),
    repoRoot,
    sourceCatalogPath: resolve(repoRoot, "contrib/i18n/en.json5"),
  };
}

/**
 * Returns whether one filename matches the locale catalog naming convention.
 */
export function isLocaleCatalogFilename(name: string): boolean {
  return LOCALE_CATALOG_NAME.test(name);
}
