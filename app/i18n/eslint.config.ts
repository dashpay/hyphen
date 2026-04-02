import repoEslintRules from "@repo/eslint-rules";
import type { ESLint } from "eslint";
import { defineConfig } from "eslint/config";

const repoPlugin = repoEslintRules as unknown as ESLint.Plugin;

export default defineConfig([
  ...repoEslintRules.configs.common,
  {
    plugins: {
      "@repo": repoPlugin,
    },
    rules: {
      "@repo/enforce-layer-imports": "error",
      "@repo/no-canonical-tr-keys": "error",
    },
  },
]);
