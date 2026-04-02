import { expect, test } from "playwright/test";
import type { APIRequestContext } from "playwright/test";

interface ValidatorSummary {
  readonly proTxHash: string;
}

interface ValidatorListResponse {
  readonly value: readonly ValidatorSummary[];
}

async function readJson<T>(request: APIRequestContext, path: string): Promise<T> {
  const response = await request.get(`http://127.0.0.1:3080${path}`);
  expect(response.ok()).toBe(true);
  return (await response.json()) as T;
}

test("renders validator detail lhs with cards, network, addresses, and last paid", async ({ page, request }) => {
  const validators = await readJson<ValidatorListResponse>(request, "/api/v1/validators?limit=1");
  test.skip(validators.value.length === 0, "no validators available");

  const hash = validators.value[0]?.proTxHash ?? null;
  if (hash == null) {
    test.skip(true, "no validator hash available");
    return;
  }

  const validatorHash = hash;

  await page.goto(`/validator?hash=${validatorHash}`);

  const main = page.locator("main");

  await expect(main.getByText("Validator", { exact: true })).toBeVisible();
  await expect(main.getByText("PoSe Score", { exact: true })).toBeVisible();
  await expect(main.getByText("Network", { exact: true })).toBeVisible();
  await expect(main.getByText("Addresses", { exact: true })).toBeVisible();

  const statCards = main.locator(".validator-stat-grid").first().locator(".hy-reference-stat-card");
  await expect(statCards).toHaveCount(3);

  const registeredCards = main.locator(".validator-stat-grid--two").first().locator(".hy-reference-stat-card");
  await expect(registeredCards).toHaveCount(2);
  await expect(registeredCards.getByText("Registered", { exact: true })).toBeVisible();
  await expect(registeredCards.getByText("Last Paid", { exact: true })).toBeVisible();

  const networkCells = main.locator(".network-cell");
  await expect(networkCells.first()).toBeVisible();

  await expect(main.getByText("Owner", { exact: true })).toBeVisible();
  await expect(main.getByText("Operator", { exact: true })).toBeVisible();
  await expect(main.getByText("Payout", { exact: true })).toBeVisible();
  await expect(main.getByText("Payee", { exact: true })).toBeVisible();
  await expect(main.getByText("Voting", { exact: true })).toBeVisible();
});

test("keeps validator summary above history on mobile and uses page-owned scroll", async ({ page, request }) => {
  const validators = await readJson<ValidatorListResponse>(request, "/api/v1/validators?limit=1");
  test.skip(validators.value.length === 0, "no validators available");

  const hash = validators.value[0]?.proTxHash ?? null;
  if (hash == null) {
    test.skip(true, "no validator hash available");
    return;
  }

  await page.setViewportSize({ width: 390, height: 1000 });
  await page.goto(`/validator?hash=${hash}`);

  const frame = page.locator(".page-frame");
  const summaryPanel = page.locator(".validator-summary .panel").first();
  const historyPanel = page.locator(".validator-history .panel").first();
  const historyScrollBody = historyPanel.locator(".scroll-body").first();

  await expect(frame).toBeVisible();
  await expect(summaryPanel).toBeVisible();
  await expect(historyPanel).toBeVisible();
  await expect(historyScrollBody).toBeVisible();

  const [summaryBox, historyBox] = await Promise.all([summaryPanel.boundingBox(), historyPanel.boundingBox()]);
  expect(summaryBox).not.toBeNull();
  expect(historyBox).not.toBeNull();
  expect((summaryBox?.y ?? 0) < (historyBox?.y ?? 0)).toBe(true);

  const hasOverflow = await frame.evaluate((node) => node.scrollHeight > node.clientHeight);
  if (!hasOverflow) {
    return;
  }

  await frame.hover();
  await page.mouse.wheel(0, 1200);

  await expect
    .poll(async () => {
      return frame.evaluate((node) => node.scrollTop);
    })
    .toBeGreaterThan(0);

  await expect
    .poll(async () => {
      return historyScrollBody.evaluate((node) => node.scrollTop);
    })
    .toBe(0);
});

test("keeps the last validator history row reachable above the dock on mobile", async ({ page, request }) => {
  const validators = await readJson<ValidatorListResponse>(request, "/api/v1/validators?limit=1");
  test.skip(validators.value.length === 0, "no validators available");

  const hash = validators.value[0]?.proTxHash ?? null;
  if (hash == null) {
    test.skip(true, "no validator hash available");
    return;
  }

  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto(`/validator?hash=${hash}`);

  const frame = page.locator(".page-frame");
  const dock = page.locator(".mobile-dock");
  const rows = page.locator(".validator-history .hy-reference-list .list-row");

  const rowCount = await rows.count();
  test.skip(rowCount === 0, "no validator history rows available");

  const lastRow = rows.nth(rowCount - 1);
  await lastRow.scrollIntoViewIfNeeded();

  const [rowBox, dockBox] = await Promise.all([lastRow.boundingBox(), dock.boundingBox()]);
  expect(rowBox).not.toBeNull();
  expect(dockBox).not.toBeNull();
  expect((rowBox?.y ?? 0) + (rowBox?.height ?? 0) <= (dockBox?.y ?? Number.POSITIVE_INFINITY)).toBe(true);

  await expect
    .poll(async () => {
      return frame.evaluate((node) => node.scrollTop);
    })
    .toBeGreaterThan(0);
});
