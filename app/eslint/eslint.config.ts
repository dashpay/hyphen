import repoEslintRules from "./src/index.ts";
import { defineConfig } from "eslint/config";

export default defineConfig([...repoEslintRules.configs.common]);
