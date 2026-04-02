import { expect, test, type Locator } from "playwright/test";

async function expectVerticalRowRhythm(locator: Locator, selectors: readonly string[]): Promise<void> {
  const boxes = await Promise.all(
    selectors.map(async (selector) => {
      const box = await locator.locator(selector).boundingBox();
      expect(box).not.toBeNull();
      return box;
    })
  );

  for (let index = 1; index < boxes.length; index += 1) {
    const previousBox = boxes[index - 1];
    const nextBox = boxes[index];
    expect(previousBox).not.toBeNull();
    expect(nextBox).not.toBeNull();
    expect((nextBox?.y ?? 0) > (previousBox?.y ?? 0)).toBe(true);
  }
}

test.describe("compact row layout integrity", () => {
  test("keeps overview latest transaction rows stacked cleanly with ellipsis", async ({ page }) => {
    await page.setViewportSize({ width: 560, height: 1200 });
    await page.goto("/");

    const firstRow = page.locator(".tx-panel .tx-row").first();
    test.skip((await firstRow.count()) === 0, "no overview pending rows available in the current dataset");
    await expect(firstRow).toBeVisible();

    const rowBands = [".detail-row-top", ".hy-compact-row-bottom"];
    const flowBand = firstRow.locator(".hy-compact-row-flow");

    if ((await flowBand.count()) > 0) {
      rowBands.push(".hy-compact-row-flow");
    }

    await expectVerticalRowRhythm(firstRow, rowBands);

    const hashLink = firstRow.locator(".detail-row-top .hy-compact-row-link").first();
    await expect(hashLink).toBeVisible();

    const hashMetrics = await hashLink.evaluate((node) => {
      const element = node as HTMLElement;
      const style = globalThis.getComputedStyle(element);
      return {
        overflow: style.overflow,
        textOverflow: style.textOverflow,
        whiteSpace: style.whiteSpace,
      };
    });

    expect(hashMetrics.overflow).toBe("hidden");
    expect(hashMetrics.textOverflow).toBe("ellipsis");
    expect(hashMetrics.whiteSpace).toBe("nowrap");

    const amountText = firstRow.locator(".hy-compact-row-side .text").first();
    const amountLabel = ((await amountText.textContent()) ?? "").trim();
    expect(amountLabel).not.toMatch(/^[-.\u2026]+$/u);
  });

  test("keeps overview latest block rows stacked cleanly with ellipsis", async ({ page }) => {
    await page.setViewportSize({ width: 560, height: 1200 });
    await page.goto("/");

    const firstRow = page.locator(".block-panel .block-row").first();
    await expect(firstRow).toBeVisible();

    await expectVerticalRowRhythm(firstRow, [".row-top", ".hy-compact-row-bottom"]);

    const hashText = firstRow.locator(".hash-cell .text").first();
    const hashMetrics = await hashText.evaluate((node) => {
      const element = node as HTMLElement;
      const style = globalThis.getComputedStyle(element);
      return {
        overflow: style.overflow,
        textOverflow: style.textOverflow,
        whiteSpace: style.whiteSpace,
      };
    });

    expect(hashMetrics.overflow).toBe("hidden");
    expect(hashMetrics.textOverflow).toBe("ellipsis");
    expect(hashMetrics.whiteSpace).toBe("nowrap");
  });

  test("keeps block transaction compact rows single-line in the top detail band", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 1200 });
    await page.goto("/");
    await page.locator(".block-panel .block-row").first().click();
    await expect(page).toHaveURL(/\/block\?hash=/);

    const firstRow = page.locator(".tx-panel .tx-row").first();
    test.skip((await firstRow.count()) === 0, "no block transaction rows available in the current dataset");
    await expect(firstRow).toBeVisible();

    const topBand = firstRow.locator(".detail-row-top");
    const pill = topBand.locator(".hy-tag").first();
    const link = topBand.locator(".hy-compact-row-link").first();

    const [pillBox, linkBox] = await Promise.all([pill.boundingBox(), link.boundingBox()]);
    expect(pillBox).not.toBeNull();
    expect(linkBox).not.toBeNull();
    expect(Math.abs((pillBox?.y ?? 0) - (linkBox?.y ?? 0)) < 6).toBe(true);
  });
});
