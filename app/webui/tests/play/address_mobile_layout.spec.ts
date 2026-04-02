import { expect, test, type Page } from "playwright/test";

async function findScrollableAddress(page: Page): Promise<string | null> {
  const response = await page.request.get("http://127.0.0.1:3080/api/v1/balances?limit=20");
  expect(response.ok()).toBe(true);
  const body = (await response.json()) as {
    readonly value: readonly { readonly address: string; readonly txCount: number }[];
  };

  for (const entry of body.value) {
    if (entry.txCount > 20) {
      return entry.address;
    }
  }

  return body.value[0]?.address ?? null;
}

test.describe("address mobile layout", () => {
  test("keeps address summary above transactions and uses page-owned scroll in dock mode", async ({ page }) => {
    const address = await findScrollableAddress(page);
    test.skip(address == null, "no address discovered");
    if (address == null) {
      return;
    }

    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto(`/address?id=${address}`);

    const frame = page.locator(".page-frame");
    const summaryPanel = page.locator(".address-summary .panel").first();
    const txPanel = page.locator(".address-transactions .tx-panel .panel").first();
    const txScrollBody = txPanel.locator(".scroll-body").first();

    await expect(frame).toBeVisible();
    await expect(summaryPanel).toBeVisible();
    await expect(txPanel).toBeVisible();
    await expect(txScrollBody).toBeVisible();

    const [summaryBox, txBox] = await Promise.all([summaryPanel.boundingBox(), txPanel.boundingBox()]);
    expect(summaryBox).not.toBeNull();
    expect(txBox).not.toBeNull();
    expect((summaryBox?.y ?? 0) < (txBox?.y ?? 0)).toBe(true);

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
        return txScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });
});
