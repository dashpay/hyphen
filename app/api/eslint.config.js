import { defineConfig } from "eslint/config";
import repoEslint from "@repo/eslint-rules";

export default defineConfig([
  ...repoEslint.configs.common,
  {
    ignores: ["dist/**", "node_modules/**"],
  },
  {
    languageOptions: {
      parserOptions: {
        projectService: {
          allowDefaultProject: ["eslint.config.js", "vitest.config.ts"],
        },
      },
    },
    plugins: {
      "@repo": repoEslint,
    },
    rules: {
      "@repo/enforce-layer-imports": "error",
    },
  },
]);
