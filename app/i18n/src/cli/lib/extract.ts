import { readFile, readdir } from "node:fs/promises";
import { join, resolve } from "node:path";

import { parse as parseSvelte } from "svelte/compiler";
import ts from "typescript";

const TS_EXTENSIONS = new Set([".ts", ".svelte"]);
const IGNORED_DIRS = new Set([".svelte-kit", "build", "dist", "node_modules", "test-results"]);

/**
 * Extracted set of English source strings discovered in the repository.
 */
export interface ExtractedCatalog {
  readonly sources: readonly string[];
}

/**
 * Options controlling one extraction run.
 */
export interface ExtractOptions {
  readonly repoRoot: string;
}

interface SourceFileContent {
  readonly path: string;
  readonly text: string;
}

interface SvelteScriptBlock {
  readonly content: {
    readonly end: number;
    readonly start: number;
  };
}

interface EstreeIdentifierNode {
  readonly type: "Identifier";
  readonly name: string;
}

interface EstreeLiteralNode {
  readonly type: "Literal";
  readonly value: unknown;
}

interface EstreeCallExpressionNode {
  readonly type: "CallExpression";
  readonly callee: unknown;
  readonly arguments: readonly unknown[];
}

function isSvelteScriptBlock(value: unknown): value is SvelteScriptBlock {
  if (typeof value !== "object" || value == null) {
    return false;
  }

  if (!("content" in value) || typeof value.content !== "object" || value.content == null) {
    return false;
  }

  return (
    "start" in value.content &&
    typeof value.content.start === "number" &&
    "end" in value.content &&
    typeof value.content.end === "number"
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value != null;
}

function isEstreeIdentifierNode(value: unknown): value is EstreeIdentifierNode {
  return isRecord(value) && value["type"] === "Identifier" && typeof value["name"] === "string";
}

function isEstreeLiteralNode(value: unknown): value is EstreeLiteralNode {
  return isRecord(value) && value["type"] === "Literal";
}

function isEstreeCallExpressionNode(value: unknown): value is EstreeCallExpressionNode {
  return (
    isRecord(value) && value["type"] === "CallExpression" && Array.isArray(value["arguments"]) && "callee" in value
  );
}

function getExtension(path: string): string {
  const lastDot = path.lastIndexOf(".");
  return lastDot >= 0 ? path.slice(lastDot) : "";
}

async function listSourceFiles(root: string): Promise<readonly string[]> {
  const files: string[] = [];
  const queue = [root];

  while (queue.length > 0) {
    const current = queue.shift();

    if (current == null) {
      continue;
    }

    const entries = await readdir(current, { withFileTypes: true });

    for (const entry of entries) {
      const nextPath = join(current, entry.name);

      if (entry.isDirectory()) {
        if (!IGNORED_DIRS.has(entry.name)) {
          queue.push(nextPath);
        }
        continue;
      }

      if (TS_EXTENSIONS.has(getExtension(entry.name))) {
        files.push(nextPath);
      }
    }
  }

  return files.sort((left, right) => left.localeCompare(right));
}

function collectImportedTrNames(sourceFile: ts.SourceFile): Set<string> {
  const names = new Set<string>();

  sourceFile.forEachChild((node): void => {
    if (!ts.isImportDeclaration(node) || node.importClause == null) {
      return;
    }

    if (!ts.isStringLiteral(node.moduleSpecifier) || node.moduleSpecifier.text !== "@repo/hyphen-i18n") {
      return;
    }

    const namedBindings = node.importClause.namedBindings;
    if (namedBindings == null || !ts.isNamedImports(namedBindings)) {
      return;
    }

    for (const element of namedBindings.elements) {
      const importedName = element.propertyName?.text ?? element.name.text;
      if (importedName === "tr") {
        names.add(element.name.text);
      }
    }
  });

  return names;
}

function getLiteralTranslation(node: ts.CallExpression, path: string): string {
  if (node.arguments.length < 1) {
    throw new Error("Expected tr() to receive at least one argument in " + path);
  }

  const argument = node.arguments[0];

  if (argument == null) {
    throw new Error("Expected tr() to receive at least one argument in " + path);
  }

  if (ts.isStringLiteral(argument) || ts.isNoSubstitutionTemplateLiteral(argument)) {
    return argument.text;
  }

  throw new Error("Expected tr() to use a string literal in " + path);
}

function getLiteralTranslationFromEstree(node: EstreeCallExpressionNode, path: string): string {
  if (node.arguments.length < 1) {
    throw new Error("Expected tr() to receive at least one argument in " + path);
  }

  const argument = node.arguments[0];

  if (isEstreeLiteralNode(argument) && typeof argument.value === "string") {
    return argument.value;
  }

  throw new Error("Expected tr() to use a string literal in " + path);
}

function extractFromTypeScript(content: SourceFileContent): readonly string[] {
  const sourceFile = ts.createSourceFile(content.path, content.text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const importedTrNames = collectImportedTrNames(sourceFile);
  const sources: string[] = [];

  if (importedTrNames.size === 0) {
    return sources;
  }

  function visit(node: ts.Node): void {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && importedTrNames.has(node.expression.text)) {
      sources.push(getLiteralTranslation(node, content.path));
    }

    node.forEachChild(visit);
  }

  visit(sourceFile);
  return sources;
}

function extractFromSvelte(content: SourceFileContent): readonly string[] {
  const parsed = parseSvelte(content.text);
  const sources: string[] = [];
  const scripts = [parsed["instance"], parsed["module"]].filter((script): script is SvelteScriptBlock =>
    isSvelteScriptBlock(script)
  );
  const importedTrNames = new Set<string>();

  for (const script of scripts) {
    const scriptText = content.text.slice(script.content.start, script.content.end);
    const sourceFile = ts.createSourceFile(content.path, scriptText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    const scriptImportedTrNames = collectImportedTrNames(sourceFile);

    for (const importedName of scriptImportedTrNames) {
      importedTrNames.add(importedName);
    }

    if (scriptImportedTrNames.size === 0) {
      continue;
    }

    function visit(node: ts.Node): void {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        scriptImportedTrNames.has(node.expression.text)
      ) {
        sources.push(getLiteralTranslation(node, content.path));
      }

      node.forEachChild(visit);
    }

    visit(sourceFile);
  }

  if (importedTrNames.size === 0) {
    return sources;
  }

  function visitTemplateNode(node: unknown): void {
    if (Array.isArray(node)) {
      for (const child of node) {
        visitTemplateNode(child);
      }
      return;
    }

    if (!isRecord(node)) {
      return;
    }

    if (
      isEstreeCallExpressionNode(node) &&
      isEstreeIdentifierNode(node.callee) &&
      importedTrNames.has(node.callee.name)
    ) {
      sources.push(getLiteralTranslationFromEstree(node, content.path));
    }

    for (const value of Object.values(node)) {
      visitTemplateNode(value);
    }
  }

  visitTemplateNode(parsed["html"]);

  return sources;
}

/**
 * Extracts the complete set of `tr("...")` source strings from TypeScript and
 * Svelte source files under the repository root.
 */
export async function extractCatalog(options: ExtractOptions): Promise<ExtractedCatalog> {
  const appRoot = resolve(options.repoRoot, "app");
  const files = await listSourceFiles(appRoot);
  const sources = new Set<string>();

  for (const path of files) {
    const text = await readFile(path, "utf8");
    const content: SourceFileContent = { path, text };
    const extracted = getExtension(path) === ".svelte" ? extractFromSvelte(content) : extractFromTypeScript(content);

    for (const source of extracted) {
      sources.add(source);
    }
  }

  return {
    sources: [...sources].sort((left, right) => left.localeCompare(right)),
  };
}
