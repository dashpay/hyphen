import path from "node:path";

import { sveltekit } from "@sveltejs/kit/vite";
import Icons from "unplugin-icons/vite";
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    sourcemap: true,
  },
  css: {
    preprocessorOptions: {
      scss: {
        loadPaths: [path.resolve("src/gen"), path.resolve("../views/src/gen")],
      },
    },
  },
  plugins: [
    sveltekit(),
    Icons({
      compiler: "svelte",
    }),
  ],
  server: {
    fs: {
      strict: true,
    },
    proxy: {
      "/api": {
        target: "http://127.0.0.1:3080",
        changeOrigin: true,
      },
    },
  },
});
