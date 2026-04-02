import { readdir } from "node:fs/promises";
import { join } from "node:path";

import { readCatalogDocument, writeCatalogDocument } from "./lib/catalog.js";
import { getRepoPaths, isLocaleCatalogFilename } from "./lib/paths.js";

async function main(): Promise<void> {
  const paths = getRepoPaths();
  const entries = await readdir(paths.localeDir, { withFileTypes: true });

  for (const entry of entries) {
    if (!entry.isFile() || !isLocaleCatalogFilename(entry.name)) {
      continue;
    }

    const path = join(paths.localeDir, entry.name);
    const document = await readCatalogDocument(path);
    await writeCatalogDocument(path, document);
  }
}

void main();
