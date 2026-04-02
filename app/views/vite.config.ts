import path from "node:path";

import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    sourcemap: true,
  },
  css: {
    preprocessorOptions: {
      scss: {
        loadPaths: [path.resolve("src/gen")],
      },
    },
  },
  plugins: [sveltekit()],
  server: {
    fs: {
      strict: true,
    },
  },
});
