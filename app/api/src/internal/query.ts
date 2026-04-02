import type { RequestOptions } from "../contracts/common.js";

export function encodePathSegment(value: string): string {
  return encodeURIComponent(value);
}

export function createQuery(params: Readonly<Record<string, string | number | undefined>>): URLSearchParams {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value == null) {
      continue;
    }
    query.set(key, String(value));
  }
  return query;
}

export function buildUrl(baseUrl: URL, path: string, options?: RequestOptions): URL {
  const url = new URL(path, baseUrl);
  if (options?.query != null) {
    for (const [key, value] of options.query.entries()) {
      url.searchParams.append(key, value);
    }
  }
  return url;
}
