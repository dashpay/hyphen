import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const FIXTURE_DIR = join(fileURLToPath(new URL("..", import.meta.url)), "fixtures");

export async function loadJsonFixture(name: string): Promise<unknown> {
  const raw = await readFile(join(FIXTURE_DIR, name), "utf8");
  return JSON.parse(raw) as unknown;
}

export async function loadTextFixture(name: string): Promise<string> {
  return readFile(join(FIXTURE_DIR, name), "utf8");
}
