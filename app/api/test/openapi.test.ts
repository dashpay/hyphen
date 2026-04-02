import { describe, expect, it } from "vitest";

import { loadTextFixture } from "./support.js";

async function readOpenApiSnapshot(): Promise<string> {
  return loadTextFixture("openapi.snapshot.json");
}

describe("openapi snapshot", (): void => {
  it("stores a committed openapi dump for intentional drift review", async (): Promise<void> => {
    const snapshot = await readOpenApiSnapshot();
    const normalized = JSON.stringify(JSON.parse(snapshot) as unknown);

    expect(snapshot.length).toBeGreaterThan(0);
    expect(normalized.length).toBeGreaterThan(0);
  });

  it("matches the live openapi spec when a spec url is provided", async (): Promise<void> => {
    const specUrl = process.env["HYPHEN_API_SPEC_URL"];

    if (specUrl == null || specUrl.length === 0) {
      return;
    }

    const response = await fetch(specUrl);
    expect(response.ok).toBe(true);

    const live = JSON.stringify((await response.json()) as unknown);
    const snapshot = JSON.stringify(JSON.parse(await readOpenApiSnapshot()) as unknown);

    expect(live).toBe(snapshot);
  }, 60_000);
});
