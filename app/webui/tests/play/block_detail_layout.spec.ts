import { expect, test } from "playwright/test";

test.describe("block detail layout", () => {
  test("keeps summary above transactions on mobile and uses page-owned scroll", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto("/block?hash=00000000000000295fbe0722c81a1a284d2c5e019686e1a45fac23f135383c63");

    const frame = page.locator(".page-frame");
    const summaryPanel = page.locator(".block-summary .panel").first();
    const txPanel = page.locator(".block-transactions .panel").first();
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
