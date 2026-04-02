import { expect, test, type Page } from "playwright/test";

async function openHashRateChart(page: Page): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Expand Hash Rate chart").click();
  await expect(page.locator(".graph-overlay")).toBeVisible();
}

test.describe("chart responsive presentation", () => {
  test("uses fullscreen on mobile and overlay on desktop after resizing", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await openHashRateChart(page);

    const overlay = page.locator(".graph-overlay");
    const panel = page.locator(".graph-overlay .panel").first();
    const closeButton = page.getByLabel("Close chart");

    await expect(overlay).toHaveClass(/fullscreen/);
    await expect(closeButton).toBeVisible();

    const [mobileOverlayBox, mobilePanelBox] = await Promise.all([overlay.boundingBox(), panel.boundingBox()]);
    expect(mobileOverlayBox).not.toBeNull();
    expect(mobilePanelBox).not.toBeNull();
    expect(Math.abs((mobilePanelBox?.width ?? 0) - (mobileOverlayBox?.width ?? 0)) <= 2).toBe(true);
    expect(Math.abs((mobilePanelBox?.height ?? 0) - (mobileOverlayBox?.height ?? 0)) <= 2).toBe(true);

    await closeButton.click();
    await expect(overlay).toHaveCount(0);

    await page.setViewportSize({ width: 1280, height: 900 });
    await page.waitForTimeout(100);
    await page.getByLabel("Expand Hash Rate chart").click();
    await expect(overlay).toBeVisible();
    await expect(overlay).not.toHaveClass(/fullscreen/);

    const [desktopOverlayBox, desktopPanelBox] = await Promise.all([overlay.boundingBox(), panel.boundingBox()]);
    expect(desktopOverlayBox).not.toBeNull();
    expect(desktopPanelBox).not.toBeNull();
    expect((desktopPanelBox?.width ?? 0) < (desktopOverlayBox?.width ?? 0)).toBe(true);
    expect((desktopPanelBox?.height ?? 0) < (desktopOverlayBox?.height ?? 0)).toBe(true);
  });
});
