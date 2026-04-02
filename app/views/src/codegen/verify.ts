import fs from "node:fs/promises";
import path from "node:path";

const ROOT_DIR = path.resolve("src");
const BANNED_QUERY = /@(media|container)\s*\((max|min)-width:\s*(360|480|768|1023|1024|1440|1920)px\)/g;

async function listSourceFiles(dirPath: string): Promise<readonly string[]> {
  const entries = await fs.readdir(dirPath, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry): Promise<readonly string[]> => {
      const entryPath = path.join(dirPath, entry.name);

      if (entry.isDirectory()) {
        if (entry.name === "gen") {
          return [];
        }

        return listSourceFiles(entryPath);
      }

      if (!/\.(css|scss|svelte)$/.test(entry.name)) {
        return [];
      }

      return [entryPath];
    })
  );

  return nested.flat();
}

function findViolations(source: string, filePath: string): readonly string[] {
  const violations: string[] = [];

  for (const match of source.matchAll(BANNED_QUERY)) {
    const offset = match.index;
    const line = source.slice(0, offset).split("\n").length;
    violations.push(`${filePath}:${String(line)} uses raw breakpoint query ${match[0]}`);
  }

  return violations;
}

async function main(): Promise<void> {
  const files = await listSourceFiles(ROOT_DIR);
  const violations = (
    await Promise.all(
      files.map(async (filePath): Promise<readonly string[]> => {
        const source = await fs.readFile(filePath, "utf8");
        return findViolations(source, filePath);
      })
    )
  ).flat();

  if (violations.length === 0) {
    return;
  }

  throw new Error(`Token verification failed:\n${violations.join("\n")}`);
}

void main();
