import { parse } from "smol-toml";

import { DEFAULT_WEBUI_CONFIG, normalizeWebUiConfig, type WebUiConfig } from "./schema";

export const WEBUI_CONFIG_PATH = "/webui.conf";

let config: WebUiConfig = $state(DEFAULT_WEBUI_CONFIG);
let loading = $state(false);
let loaded = $state(false);
let error: string | null = $state(null);

/** Returns the current normalized webui config state. */
export function getWebUiConfig(): WebUiConfig {
  return config;
}

/** Returns true while the config file is still loading. */
export function webUiConfigLoading(): boolean {
  return loading;
}

/** Returns true after config load has completed successfully. */
export function webUiConfigLoaded(): boolean {
  return loaded;
}

/** Returns the latest config load error, if one exists. */
export function webUiConfigError(): string | null {
  return error;
}

/** Loads and normalizes webui.conf, falling back to defaults on 404. */
export async function loadWebUiConfig(path: string = WEBUI_CONFIG_PATH): Promise<void> {
  if (loading || loaded) {
    return;
  }

  loading = true;
  error = null;

  try {
    const response = await fetch(path);

    if (!response.ok) {
      if (response.status === 404) {
        config = DEFAULT_WEBUI_CONFIG;
        loaded = true;
        return;
      }

      throw new Error("Failed to fetch webui.conf: " + String(response.status));
    }

    const rawText = await response.text();
    const rawConfig = parse(rawText) as unknown;

    config = normalizeWebUiConfig(rawConfig);
    loaded = true;
  } catch (caught: unknown) {
    error = caught instanceof Error ? caught.message : String(caught);
    config = DEFAULT_WEBUI_CONFIG;
  } finally {
    loading = false;
  }
}
