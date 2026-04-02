/**
 * Shared ESLint rules for repository-local architectural constraints.
 */

import { readFileSync } from "node:fs";

import eslint from "@eslint/js";
import type { Linter, Rule } from "eslint";
import prettierPlugin from "eslint-plugin-prettier/recommended";
import JSON5 from "json5";
import tseslint from "typescript-eslint";

interface LocaleMetaConfig {
  readonly blocklist?: readonly string[];
}

type LayerName =
  | "api-package"
  | "views-package"
  | "webui-routes"
  | "webui-shared-views"
  | "webui-domain-views"
  | "webui-domain-data"
  | "i18n-package"
  | "other";

const EXCLUSIONS_PATH = new URL("../../../contrib/i18n/meta.json5", import.meta.url);
const canonicalTranslationTerms = readCanonicalTranslationTerms();

function isExclusionsConfig(value: unknown): value is LocaleMetaConfig {
  if (typeof value !== "object" || value == null || Array.isArray(value)) {
    return false;
  }

  if (!("blocklist" in value) || value.blocklist == null) {
    return true;
  }

  return Array.isArray(value.blocklist) && value.blocklist.every((term): boolean => typeof term === "string");
}

function readCanonicalTranslationTerms(): ReadonlySet<string> {
  const raw = readFileSync(EXCLUSIONS_PATH, "utf8");
  const parsed: unknown = JSON5.parse(raw);

  if (!isExclusionsConfig(parsed)) {
    throw new Error("Expected meta.json5 to contain an object");
  }

  return new Set(parsed.blocklist ?? []);
}

function normalizePath(filename: string): string {
  return filename.split("\\").join("/");
}

function getLayerName(filename: string): LayerName {
  const normalized = normalizePath(filename);

  if (normalized.includes("/app/i18n/src/")) {
    return "i18n-package";
  }

  if (normalized.includes("/app/api/src/")) {
    return "api-package";
  }

  if (normalized.includes("/app/views/src/")) {
    return "views-package";
  }

  if (normalized.includes("/app/webui/src/routes/")) {
    return "webui-routes";
  }

  if (normalized.includes("/app/webui/src/lib/views/")) {
    return "webui-shared-views";
  }

  if (
    /\/app\/webui\/src\/lib\/(?:address|balances|block|mempool|network|proposal|tx|validator)\/views\//.test(normalized)
  ) {
    return "webui-domain-views";
  }

  if (/\/app\/webui\/src\/lib\/(?:address|balances|block|mempool|network|proposal|tx|validator)\//.test(normalized)) {
    return "webui-domain-data";
  }

  return "other";
}

function isRelativeImport(source: string): boolean {
  return source.startsWith("./") || source.startsWith("../");
}

function isExternalImport(source: string): boolean {
  return (
    source.startsWith("node:") ||
    source.startsWith("svelte") ||
    source.startsWith("@sveltejs/") ||
    source.startsWith("@typescript-eslint/") ||
    source.startsWith("@unovis/") ||
    source.startsWith("virtual:") ||
    (!source.startsWith("@repo/") && !source.startsWith("$lib"))
  );
}

function isAllowedImport(layer: LayerName, source: string): boolean {
  if (isRelativeImport(source) || isExternalImport(source)) {
    return true;
  }

  switch (layer) {
    case "views-package":
      return source === "@repo/hyphen-i18n" || source === "$lib" || source.startsWith("$lib/");
    case "api-package":
      return !source.startsWith("@repo/") && !source.startsWith("$lib");
    case "webui-routes":
      return (
        source === "@repo/hyphen-views" ||
        source === "@repo/hyphen-i18n" ||
        source === "$lib" ||
        source.startsWith("$lib/")
      );
    case "webui-shared-views":
      return (
        source === "@repo/hyphen-views" ||
        source === "@repo/hyphen-i18n" ||
        source === "$lib" ||
        source === "$lib/HyUtil" ||
        source === "$lib/views" ||
        source === "$lib/icons" ||
        source === "$lib/anim" ||
        source === "$lib/api" ||
        source.startsWith("$lib/data/") ||
        source.startsWith("$lib/config/")
      );
    case "webui-domain-views":
      return (
        source === "@repo/hyphen-views" ||
        source === "@repo/hyphen-i18n" ||
        source === "$lib" ||
        source === "$lib/HyUtil" ||
        source === "$lib/views" ||
        source === "$lib/icons" ||
        source === "$lib/anim" ||
        source === "$lib/api" ||
        source.startsWith("$lib/data/") ||
        source.startsWith("$lib/config/") ||
        source.startsWith("$lib/address/") ||
        source.startsWith("$lib/balances/") ||
        source.startsWith("$lib/block/") ||
        source.startsWith("$lib/mempool/") ||
        source.startsWith("$lib/network/") ||
        source.startsWith("$lib/proposal/") ||
        source.startsWith("$lib/tx/") ||
        source.startsWith("$lib/validator/")
      );
    case "webui-domain-data":
      return (
        source === "@repo/hyphen-i18n" ||
        source === "$lib" ||
        source === "$lib/HyUtil" ||
        source === "$lib/api" ||
        source.startsWith("$lib/data/") ||
        source.startsWith("$lib/config/") ||
        source.startsWith("$lib/address/") ||
        source.startsWith("$lib/balances/") ||
        source.startsWith("$lib/block/") ||
        source.startsWith("$lib/mempool/") ||
        source.startsWith("$lib/network/") ||
        source.startsWith("$lib/proposal/") ||
        source.startsWith("$lib/tx/") ||
        source.startsWith("$lib/validator/")
      );
    case "i18n-package":
      return !source.startsWith("@repo/") && !source.startsWith("$lib");
    case "other":
      return true;
  }
}

function describeRestrictedSource(source: string): string {
  if (source.startsWith("@repo/")) {
    return "local workspace package";
  }

  if (source.startsWith("$lib")) {
    return "webui local alias";
  }

  return "restricted import";
}

const enforceLayerImportsRule: Rule.RuleModule = {
  meta: {
    type: "problem",
    docs: {
      description: "enforce the repository-local layer import hierarchy",
    },
    schema: [],
  },
  create(context: Rule.RuleContext): Rule.RuleListener {
    const filename = context.filename;
    const layer = getLayerName(filename);

    return {
      ImportDeclaration(node): void {
        if (typeof node.source.value !== "string") {
          return;
        }

        const source = node.source.value;
        if (isAllowedImport(layer, source)) {
          return;
        }

        context.report({
          node: node.source,
          message: "{{kind}} is not allowed from {{layer}}.",
          data: {
            kind: describeRestrictedSource(source),
            layer,
          },
        });
      },
    };
  },
};

const noCanonicalTrKeysRule: Rule.RuleModule = {
  meta: {
    type: "problem",
    docs: {
      description: "disallow tr() on canonical glossary terms from meta.json5",
    },
    schema: [],
  },
  create(context: Rule.RuleContext): Rule.RuleListener {
    return {
      CallExpression(node): void {
        if (node.callee.type !== "Identifier" || node.callee.name !== "tr") {
          return;
        }

        const [firstArgument] = node.arguments;
        if (firstArgument?.type !== "Literal" || typeof firstArgument.value !== "string") {
          return;
        }

        if (!canonicalTranslationTerms.has(firstArgument.value)) {
          return;
        }

        context.report({
          node: firstArgument,
          message:
            "Canonical glossary terms from contrib/i18n/meta.json5 must not be wrapped in tr(); keep them literal or structural.",
        });
      },
    };
  },
};

interface RepoEslintPlugin {
  readonly configs: {
    readonly common: Linter.Config[];
  };
  readonly rules: Record<string, Rule.RuleModule>;
}

const commonConfigs: Linter.Config[] = [
  eslint.configs.recommended,
  prettierPlugin,
  ...tseslint.configs.strictTypeChecked,
  {
    ignores: ["dist/**", "node_modules/**"],
  },
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
      },
    },
    plugins: {
      "@typescript-eslint": tseslint.plugin,
    },
    rules: {
      "@typescript-eslint/await-thenable": "error",
      "@typescript-eslint/consistent-type-definitions": ["error", "interface"],
      "@typescript-eslint/consistent-type-imports": "error",
      "@typescript-eslint/explicit-function-return-type": "error",
      "@typescript-eslint/explicit-module-boundary-types": "error",
      "@typescript-eslint/no-confusing-void-expression": "error",
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      "@typescript-eslint/no-non-null-assertion": "error",
      "@typescript-eslint/no-unnecessary-condition": "error",
      "@typescript-eslint/no-unnecessary-type-assertion": "error",
      "@typescript-eslint/no-unsafe-argument": "error",
      "@typescript-eslint/no-unsafe-assignment": "error",
      "@typescript-eslint/no-unsafe-call": "error",
      "@typescript-eslint/no-unsafe-member-access": "error",
      "@typescript-eslint/no-unsafe-return": "error",
      "@typescript-eslint/prefer-nullish-coalescing": "error",
      "@typescript-eslint/prefer-optional-chain": "error",
      "@typescript-eslint/prefer-readonly": "error",
      "@typescript-eslint/require-await": "error",
      "@typescript-eslint/switch-exhaustiveness-check": "error",
    },
  },
];

const plugin: RepoEslintPlugin = {
  configs: {
    common: commonConfigs,
  },
  rules: {
    "enforce-layer-imports": enforceLayerImportsRule,
    "no-canonical-tr-keys": noCanonicalTrKeysRule,
  },
};

export default plugin;
