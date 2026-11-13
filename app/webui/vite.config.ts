import path from "node:path";
import process from "node:process";

import { sveltekit } from "@sveltejs/kit/vite";
import Icons from "unplugin-icons/vite";
import { defineConfig } from "vite";

const envApiBaseUrl = process.env["API_BASE_URL"]?.trim() ?? "";
const rawApiBaseUrl = envApiBaseUrl === "" ? "http://127.0.0.1:3080" : envApiBaseUrl;
const apiBaseUrl = new URL(rawApiBaseUrl);
const apiProxyPrefix = apiBaseUrl.pathname.replace(/\/$/u, "");

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
        target: apiBaseUrl.origin,
        changeOrigin: true,
        rewrite: (requestPath: string): string => {
          if (apiProxyPrefix.length === 0 || apiProxyPrefix === "/") {
            return requestPath;
          }

          return requestPath.replace(/^\/api/u, apiProxyPrefix);
        },
      },
    },
  },
});
