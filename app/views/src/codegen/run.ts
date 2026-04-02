import fs from "node:fs/promises";
import path from "node:path";

import chokidar from "chokidar";
import JSON5 from "json5";

interface ViewBreakpoints {
  readonly narrowPhone: number;
  readonly phone: number;
  readonly tablet: number;
  readonly laptop: number;
  readonly desktop: number;
  readonly ultrawide: number;
}

interface ViewColors {
  readonly base: Readonly<Record<string, string>>;
  readonly light: Readonly<Record<string, string>>;
}

interface ViewLayout {
  readonly spaces: Readonly<Record<string, string>>;
  readonly radii: Readonly<Record<string, string>>;
  readonly shadows: Readonly<Record<string, string>>;
}

interface ViewMotion {
  readonly easing: Readonly<Record<string, string>>;
  readonly duration: Readonly<Record<string, string>>;
}

interface ViewFonts {
  readonly families: Readonly<Record<string, string>>;
  readonly textScale: Readonly<Record<string, number>>;
  readonly fontWeight: Readonly<Record<string, number>>;
  readonly localeOverrides: readonly {
    readonly selector: string;
    readonly vars: Readonly<Record<string, string>>;
  }[];
}

interface ViewTokenBundle {
  readonly breakpoints: ViewBreakpoints;
  readonly colors: ViewColors;
  readonly layout: ViewLayout;
  readonly motion: ViewMotion;
  readonly fonts: ViewFonts;
}

const TOKEN_DIR = path.resolve("src/tokens");
const OUTPUT_DIR = path.resolve("src/gen");

async function readJson5File<TValue>(fileName: string): Promise<TValue> {
  const sourcePath = path.join(TOKEN_DIR, fileName);
  const raw = await fs.readFile(sourcePath, "utf8");
  const parsed: unknown = JSON5.parse(raw);
  return parsed as TValue;
}

function renderCssVariables(entries: Readonly<Record<string, string>>, prefix = ""): string {
  return Object.entries(entries)
    .map(([key, value]): string => `  --${prefix}${key}: ${value};`)
    .join("\n");
}

function renderTextScaleVariables(scale: Readonly<Record<string, number>>): string {
  return Object.entries(scale)
    .map(([key, value]): string => `  --text-${key.replace("+", "-plus")}: ${String(value)}px;`)
    .join("\n");
}

function renderThemeCss(bundle: ViewTokenBundle): string {
  const fontPrimary = bundle.fonts.families["primary"];
  const fontMono = bundle.fonts.families["mono"];

  if (fontPrimary == null || fontMono == null) {
    throw new Error("View font tokens must include primary and mono families.");
  }

  const breakpointVars = Object.entries(bundle.breakpoints)
    .map(([key, value]): string => `  --vw-breakpoint-${toKebabCase(key)}: ${String(value)}px;`)
    .join("\n");

  const localeBlocks = bundle.fonts.localeOverrides
    .map((override): string => `${override.selector} {\n${renderCssVariables(override.vars)}\n}`)
    .join("\n\n");

  return [
    ":root {",
    breakpointVars,
    renderCssVariables(bundle.colors.base),
    renderCssVariables(bundle.layout.spaces, "space-"),
    renderCssVariables(bundle.layout.radii, "radius-"),
    renderCssVariables(bundle.layout.shadows, "shadow-"),
    renderCssVariables(bundle.motion.easing, "ease-"),
    renderCssVariables(bundle.motion.duration, "duration-"),
    renderCssVariables({
      "font-primary": fontPrimary,
      "font-mono": fontMono,
    }),
    renderTextScaleVariables(bundle.fonts.textScale),
    "}",
    "",
    ".theme-light {",
    renderCssVariables(bundle.colors.light),
    "}",
    "",
    localeBlocks,
    "",
  ].join("\n");
}

function renderBreakpointsTs(breakpoints: ViewBreakpoints): string {
  return [
    "export const VIEW_BREAKPOINTS = " + formatTsObject(breakpoints) + " as const;",
    "",
    "export type ViewBreakpointName = keyof typeof VIEW_BREAKPOINTS;",
    "",
    "export function getViewBreakpointWidth(name: ViewBreakpointName): number {",
    "  return VIEW_BREAKPOINTS[name];",
    "}",
    "",
    "export function getViewBreakpointName(width: number): ViewBreakpointName {",
    "  if (width <= VIEW_BREAKPOINTS.narrowPhone) {",
    '    return "narrowPhone";',
    "  }",
    "",
    "  if (width <= VIEW_BREAKPOINTS.phone) {",
    '    return "phone";',
    "  }",
    "",
    "  if (width <= VIEW_BREAKPOINTS.tablet) {",
    '    return "tablet";',
    "  }",
    "",
    "  if (width <= VIEW_BREAKPOINTS.laptop) {",
    '    return "laptop";',
    "  }",
    "",
    "  if (width <= VIEW_BREAKPOINTS.desktop) {",
    '    return "desktop";',
    "  }",
    "",
    '  return "ultrawide";',
    "}",
    "",
  ].join("\n");
}

function renderTokensTs(bundle: ViewTokenBundle): string {
  const fontPrimary = bundle.fonts.families["primary"];
  const fontMono = bundle.fonts.families["mono"];

  if (fontPrimary == null || fontMono == null) {
    throw new Error("View font tokens must include primary and mono families.");
  }

  return [
    'import { VIEW_BREAKPOINTS } from "./breakpoints";',
    "",
    `export const FONT_PRIMARY = ${JSON.stringify(fontPrimary)};`,
    `export const FONT_MONO = ${JSON.stringify(fontMono)};`,
    "",
    `export const TEXT_SCALE = ${formatTsObject(bundle.fonts.textScale)} as const;`,
    "",
    "export interface FontWeightScale {",
    "  readonly light: number;",
    "  readonly regular: number;",
    "  readonly medium: number;",
    "  readonly semibold: number;",
    "  readonly bold: number;",
    "  readonly extrabold: number;",
    "}",
    "",
    `export const FONT_WEIGHT: FontWeightScale = ${formatTsObject(bundle.fonts.fontWeight)};`,
    "",
    "export const VIEW_THEME = {",
    `  breakpoints: VIEW_BREAKPOINTS,`,
    `  colors: ${formatTsObject(bundle.colors)},`,
    `  layout: ${formatTsObject(bundle.layout)},`,
    `  motion: ${formatTsObject(bundle.motion)},`,
    `  fonts: ${formatTsObject(bundle.fonts)},`,
    "} as const;",
    "",
  ].join("\n");
}

function renderBreakpointsScss(breakpoints: ViewBreakpoints): string {
  const breakpointEntries = Object.entries(breakpoints)
    .map(([key, value]): string => `  ${toKebabCase(key)}: ${String(value)}px,`)
    .join("\n");

  return [
    '@use "sass:map";',
    '@use "sass:math";',
    '@use "sass:meta";',
    "",
    "$vw-breakpoints: (",
    breakpointEntries,
    ");",
    "",
    "@function vw-breakpoint($name) {",
    "  @if not map.has-key($vw-breakpoints, $name) {",
    '    @error "Unknown breakpoint: #{$name}";',
    "  }",
    "",
    "  @return map.get($vw-breakpoints, $name);",
    "}",
    "",
    "@function vw-breakpoint-below($name) {",
    "  $value: vw-breakpoint($name);",
    "",
    '  @if meta.type-of($value) == "number" and math.unit($value) == "px" {',
    "    @return $value - 1px;",
    "  }",
    "",
    '  @error "Breakpoint #{$name} must resolve to px."; ',
    "}",
    "",
  ].join("\n");
}

function renderMixinsScss(): string {
  return [
    '@use "./_views-breakpoints.scss" as breakpoints;',
    "",
    "@mixin media-up($name) {",
    "  @media (min-width: breakpoints.vw-breakpoint($name)) {",
    "    @content;",
    "  }",
    "}",
    "",
    "@mixin media-down($name) {",
    "  @media (max-width: breakpoints.vw-breakpoint($name)) {",
    "    @content;",
    "  }",
    "}",
    "",
    "@mixin media-below($name) {",
    "  @media (max-width: breakpoints.vw-breakpoint-below($name)) {",
    "    @content;",
    "  }",
    "}",
    "",
    "@mixin container-up($name) {",
    "  @container (min-width: breakpoints.vw-breakpoint($name)) {",
    "    @content;",
    "  }",
    "}",
    "",
    "@mixin container-down($name) {",
    "  @container (max-width: breakpoints.vw-breakpoint($name)) {",
    "    @content;",
    "  }",
    "}",
    "",
    "@mixin container-below($name) {",
    "  @container (max-width: breakpoints.vw-breakpoint-below($name)) {",
    "    @content;",
    "  }",
    "}",
    "",
  ].join("\n");
}

function renderTokensScss(bundle: ViewTokenBundle): string {
  return [
    "$vw-colors: " + renderScssMap(bundle.colors.base) + ";",
    "$vw-light-colors: " + renderScssMap(bundle.colors.light) + ";",
    "$vw-spaces: " + renderScssMap(bundle.layout.spaces) + ";",
    "$vw-radii: " + renderScssMap(bundle.layout.radii) + ";",
    "$vw-shadows: " + renderScssMap(bundle.layout.shadows) + ";",
    "$vw-easing: " + renderScssMap(bundle.motion.easing) + ";",
    "$vw-durations: " + renderScssMap(bundle.motion.duration) + ";",
    "$vw-fonts: " + renderScssMap(bundle.fonts.families) + ";",
    "",
  ].join("\n");
}

function renderScssMap(values: Readonly<Record<string, string | number | boolean>>): string {
  const entries = Object.entries(values)
    .map(([key, value]): string => `${JSON.stringify(toKebabCase(key))}: ${String(value)}`)
    .join(", ");

  return `(${entries})`;
}

function formatTsObject(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function toKebabCase(input: string): string {
  return input.replace(/[A-Z]/g, (letter): string => `-${letter.toLowerCase()}`);
}

async function loadBundle(): Promise<ViewTokenBundle> {
  return {
    breakpoints: await readJson5File<ViewBreakpoints>("breakpoints.json5"),
    colors: await readJson5File<ViewColors>("colors.json5"),
    layout: await readJson5File<ViewLayout>("layout.json5"),
    motion: await readJson5File<ViewMotion>("motion.json5"),
    fonts: await readJson5File<ViewFonts>("fonts.json5"),
  };
}

async function writeOutputs(): Promise<void> {
  const bundle = await loadBundle();

  await fs.mkdir(OUTPUT_DIR, { recursive: true });
  await Promise.all([
    fs.writeFile(path.join(OUTPUT_DIR, "theme.css"), renderThemeCss(bundle)),
    fs.writeFile(path.join(OUTPUT_DIR, "breakpoints.ts"), renderBreakpointsTs(bundle.breakpoints)),
    fs.writeFile(path.join(OUTPUT_DIR, "tokens.ts"), renderTokensTs(bundle)),
    fs.writeFile(path.join(OUTPUT_DIR, "_views-breakpoints.scss"), renderBreakpointsScss(bundle.breakpoints)),
    fs.writeFile(path.join(OUTPUT_DIR, "_views-mixins.scss"), renderMixinsScss()),
    fs.writeFile(path.join(OUTPUT_DIR, "_views-tokens.scss"), renderTokensScss(bundle)),
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
