import { expect, test } from "playwright/test";
import type { APIRequestContext } from "playwright/test";

interface TransactionSummary {
  readonly txid: string;
}

interface BlockSummary {
  readonly hash: string;
}

interface BlockTransactionListResponse {
  readonly value: readonly TransactionSummary[];
}

interface BlockListResponse {
  readonly value: readonly BlockSummary[];
}

async function readJson<T>(request: APIRequestContext, path: string): Promise<T> {
  const response = await request.get(`http://127.0.0.1:3080${path}`);
  expect(response.ok()).toBe(true);
  return (await response.json()) as T;
}

test.describe("transaction detail layout", () => {
  test("keeps summary above details on mobile and uses page-owned scroll", async ({ page, request }) => {
    const blocks = await readJson<BlockListResponse>(request, "/api/v1/blocks?limit=1");
    test.skip(blocks.value.length === 0, "no blocks available");

    const blockHash = blocks.value[0]?.hash ?? null;
    if (blockHash == null) {
      test.skip(true, "no block hash available");
      return;
    }

    const transactions = await readJson<BlockTransactionListResponse>(
      request,
      `/api/v1/block/${blockHash}/txs?limit=1`
    );
    test.skip(transactions.value.length === 0, "no block transactions available");

    const hash = transactions.value[0]?.txid ?? null;
    if (hash == null) {
      test.skip(true, "no transaction hash available");
      return;
    }

    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto(`/transaction?hash=${hash}`);

    const frame = page.locator(".page-frame");
    const summaryPanel = page.locator(".transaction-summary").first();
    const detailPanel = page.locator(".transaction-detail .panel").first();
    const detailScrollBody = detailPanel.locator(".scroll-body").first();

    await expect(frame).toBeVisible();
    await expect(summaryPanel).toBeVisible();
    await expect(detailPanel).toBeVisible();
    await expect(detailScrollBody).toBeVisible();

    const [summaryBox, detailBox] = await Promise.all([summaryPanel.boundingBox(), detailPanel.boundingBox()]);
    expect(summaryBox).not.toBeNull();
    expect(detailBox).not.toBeNull();
    expect((summaryBox?.y ?? 0) < (detailBox?.y ?? 0)).toBe(true);

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
        return detailScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });
});
