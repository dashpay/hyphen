import fs from "node:fs/promises";
import path from "node:path";

import chokidar from "chokidar";
import JSON5 from "json5";

interface WebUiTokenSlice {
  readonly base: Readonly<Record<string, string>>;
  readonly light?: Readonly<Record<string, string>>;
  readonly yellow?: Readonly<Record<string, string>>;
  readonly yellowLight?: Readonly<Record<string, string>>;
}

interface WebUiTokenBundle {
  readonly base: Readonly<Record<string, string>>;
  readonly light: Readonly<Record<string, string>>;
  readonly yellow: Readonly<Record<string, string>>;
  readonly yellowLight: Readonly<Record<string, string>>;
}

const TOKEN_DIR = path.resolve("src/tokens");
const OUTPUT_DIR = path.resolve("src/gen");

async function readJson5File<TValue>(fileName: string): Promise<TValue> {
  const sourcePath = path.join(TOKEN_DIR, fileName);
  const raw = await fs.readFile(sourcePath, "utf8");
  const parsed: unknown = JSON5.parse(raw);
  return parsed as TValue;
}

function renderCssVariables(entries: Readonly<Record<string, string>>): string {
  return Object.entries(entries)
    .map(([key, value]): string => `  --${key}: ${value};`)
    .join("\n");
}

function renderThemeCss(bundle: WebUiTokenBundle): string {
  const parts = [
    ":root {",
    renderCssVariables(bundle.base),
    "}",
    "",
    ".theme-light {",
    renderCssVariables(bundle.light),
    "}",
    "",
  ];

  if (Object.keys(bundle.yellow).length > 0) {
    parts.push(
      ":root.theme-yellow {",
      renderCssVariables(bundle.yellow),
      "}",
      "",
    );
  }

  if (Object.keys(bundle.yellowLight).length > 0) {
    parts.push(
      ":root.theme-yellow.theme-light {",
      renderCssVariables(bundle.yellowLight),
      "}",
      "",
    );
  }

  return parts.join("\n");
}

function renderTokensTs(bundle: WebUiTokenBundle): string {
  return [`export const WEBUI_THEME = ${JSON.stringify(bundle, null, 2)} as const;`, ""].join("\n");
}

function renderTokensScss(bundle: WebUiTokenBundle): string {
  return [
    "$hy-base-tokens: " + renderScssMap(bundle.base) + ";",
    "$hy-light-tokens: " + renderScssMap(bundle.light) + ";",
    "",
  ].join("\n");
}

function renderScssMap(values: Readonly<Record<string, string>>): string {
  const entries = Object.entries(values)
    .map(([key, value]): string => `${JSON.stringify(key)}: ${value}`)
    .join(", ");

  return `(${entries})`;
}

async function loadBundle(): Promise<WebUiTokenBundle> {
  const fileNames = (await fs.readdir(TOKEN_DIR)).filter((fileName): boolean => fileName.endsWith(".json5")).sort();
  const slices = await Promise.all(fileNames.map((fileName): Promise<WebUiTokenSlice> => readJson5File(fileName)));

  return slices.reduce<WebUiTokenBundle>(
    (bundle, slice): WebUiTokenBundle => ({
      base: {
        ...bundle.base,
        ...slice.base,
      },
      light: {
        ...bundle.light,
        ...(slice.light ?? {}),
      },
      yellow: {
        ...bundle.yellow,
        ...(slice.yellow ?? {}),
      },
      yellowLight: {
        ...bundle.yellowLight,
        ...(slice.yellowLight ?? {}),
      },
    }),
    {
      base: {},
      light: {},
      yellow: {},
      yellowLight: {},
    }
  );
}

async function writeOutputs(): Promise<void> {
  const bundle = await loadBundle();

  await fs.mkdir(OUTPUT_DIR, { recursive: true });
  await Promise.all([
    fs.writeFile(path.join(OUTPUT_DIR, "theme.css"), renderThemeCss(bundle)),
    fs.writeFile(path.join(OUTPUT_DIR, "tokens.ts"), renderTokensTs(bundle)),
    fs.writeFile(path.join(OUTPUT_DIR, "_webui-tokens.scss"), renderTokensScss(bundle)),
  ]);
}

async function main(): Promise<void> {
  await writeOutputs();

  if (!process.argv.includes("--watch")) {
    return;
  }

  const watcher = chokidar.watch(path.join(TOKEN_DIR, "*.json5"), {
    ignoreInitial: true,
  });

  watcher.on("all", (): void => {
    void writeOutputs();
  });

  await new Promise<void>(() => {
    // Keep the process alive for watch mode.
  });
}

void main();
