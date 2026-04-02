import repoEslintRules from "@repo/eslint-rules";
import svelte from "eslint-plugin-svelte";
import { defineConfig } from "eslint/config";
import svelteParser from "svelte-eslint-parser";
import tseslint from "typescript-eslint";

export default defineConfig([
  ...repoEslintRules.configs.common,
  ...svelte.configs["flat/recommended"],
  {
    ignores: ["build/**", "public/**", "src/gen/**", "vite.config.ts.timestamp*", ".svelte-kit/**"],
  },
  {
    files: ["**/*.svelte"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: {
        parser: tseslint.parser,
      },
    },
  },
  {
    languageOptions: {
      parserOptions: {
        projectService: {
          allowDefaultProject: ["eslint.config.js", "svelte.config.js"],
        },
        extraFileExtensions: [".svelte"],
      },
    },
    plugins: {
      "@repo": repoEslintRules,
    },
    rules: {
      "@repo/enforce-layer-imports": "error",
      "@repo/no-canonical-tr-keys": "error",
    },
  },
]);
