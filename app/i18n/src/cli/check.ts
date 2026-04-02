import { readdir } from "node:fs/promises";
import { join } from "node:path";
import { exit } from "node:process";

import { readCatalog } from "./lib/catalog.js";
import { getRepoPaths, isLocaleCatalogFilename } from "./lib/paths.js";

async function main(): Promise<void> {
  const paths = getRepoPaths();
  const sourceCatalog = await readCatalog(paths.sourceCatalogPath);
  const sourceKeys = Object.keys(sourceCatalog);
  const entries = await readdir(paths.localeDir, { withFileTypes: true });
  let hasErrors = false;

  for (const entry of entries) {
    if (!entry.isFile() || !isLocaleCatalogFilename(entry.name) || entry.name === "en.json5") {
      continue;
    }

    const path = join(paths.localeDir, entry.name);
    const catalog = await readCatalog(path);
    const missing = sourceKeys.filter((key): boolean => !(key in catalog));
    const stale = Object.keys(catalog).filter((key): boolean => !(key in sourceCatalog));
    const empty = Object.entries(catalog)
      .filter(([, value]): boolean => value.trim().length === 0)
      .map(([key]): string => key);
    const translatedCount = sourceKeys.filter((key): boolean => (catalog[key] ?? "").trim().length > 0).length;
    const coverage = sourceKeys.length === 0 ? 100 : Math.floor((translatedCount / sourceKeys.length) * 100);

    console.log(entry.name + ": " + String(coverage) + "% coverage");

    if (missing.length > 0) {
      hasErrors = true;
      console.error(entry.name + " missing keys:");
      for (const key of missing) {
        console.error("  - " + key);
      }
    }

    if (stale.length > 0) {
      hasErrors = true;
      console.error(entry.name + " stale keys:");
      for (const key of stale) {
        console.error("  - " + key);
      }
    }

    if (empty.length > 0) {
      hasErrors = true;
      console.error(entry.name + " empty translations:");
      for (const key of empty) {
        console.error("  - " + key);
      }
    }
  }

  if (hasErrors) {
    exit(1);
  }
}

void main();
