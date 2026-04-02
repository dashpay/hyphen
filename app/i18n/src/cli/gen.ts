import { argv, exit } from "node:process";

import { readMetaDocument, serializeJson5, writeCatalog } from "./lib/catalog.js";
import { extractCatalog } from "./lib/extract.js";
import { getRepoPaths } from "./lib/paths.js";

function toCatalog(sources: readonly string[]): Record<string, string> {
  return Object.fromEntries(sources.map((source): [string, string] => [source, source]));
}

async function main(): Promise<void> {
  const check = argv.includes("--check");
  const paths = getRepoPaths();
  const extracted = await extractCatalog({ repoRoot: paths.repoRoot });
  const meta = await readMetaDocument(paths.metaPath);
  const sourceLanguage = meta.langs.find((entry): boolean => entry.src === "en.json5");
  const sourceLanguageName = sourceLanguage?.name ?? "English";
  const nextCatalog = serializeJson5({
    meta: {
      name: sourceLanguageName,
    },
    strings: toCatalog(extracted.sources),
  });

  if (check) {
    const current = await (async (): Promise<string> => {
      try {
        const { readFile } = await import("node:fs/promises");
        return await readFile(paths.sourceCatalogPath, "utf8");
      } catch {
        return serializeJson5({
          meta: {
            name: sourceLanguageName,
          },
          strings: {},
        });
      }
    })();

    if (current !== nextCatalog) {
      console.error("en.json5 is stale. Run pnpm i18n:gen.");
      exit(1);
    }
    return;
  }

  await writeCatalog(paths.sourceCatalogPath, toCatalog(extracted.sources), sourceLanguageName);
}

void main();
