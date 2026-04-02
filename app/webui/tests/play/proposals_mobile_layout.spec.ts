import { expect, test } from "playwright/test";

test.describe("proposals mobile layout", () => {
  test("shows approval only at desktop widths and wider", async ({ page }) => {
    await page.setViewportSize({ width: 1365, height: 768 });
    await page.goto("/proposals");
    await expect(page.locator(".table-header .col-bar").first()).toBeHidden();

    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/proposals");
    await expect(page.locator(".table-header .col-bar").first()).toBeVisible();
  });

  test("does not leave an inner table lip on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/proposals");

    const proposalsPanel = page.locator(".proposals-list .panel").first();
    const scrollBody = proposalsPanel.locator(".scroll-body").first();

    await expect(proposalsPanel).toBeVisible();
    await expect(scrollBody).toBeVisible();

    const [panelBox, bodyBox] = await Promise.all([proposalsPanel.boundingBox(), scrollBody.boundingBox()]);
    expect(panelBox).not.toBeNull();
    expect(bodyBox).not.toBeNull();

    expect(Math.abs((panelBox?.x ?? 0) - (bodyBox?.x ?? 0)) <= 1).toBe(true);
    expect(Math.abs((panelBox?.width ?? 0) - 2 - (bodyBox?.width ?? 0)) <= 1).toBe(true);
  });

  test("keeps the proposal summary horizontal on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/proposals");

    const summaryBody = page.locator(".proposals-summary .summary-body").first();
    const visual = summaryBody.locator(".summary-visual").first();
    const info = summaryBody.locator(".summary-info").first();

    await expect(summaryBody).toBeVisible();
    await expect(visual).toBeVisible();
    await expect(info).toBeVisible();
    await expect(info).toContainText("Registered Nodes");
    await expect(info).toContainText("Validators Voted");
    await expect(info).toContainText("Next Superblock");

    const [visualBox, infoBox] = await Promise.all([visual.boundingBox(), info.boundingBox()]);
    expect(visualBox).not.toBeNull();
    expect(infoBox).not.toBeNull();
    expect((visualBox?.x ?? 0) < (infoBox?.x ?? 0)).toBe(true);
  });

  test("uses page scroll on mobile and local table scroll on desktop", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/proposals");

    const mobileFrame = page.locator(".page-frame");
    const mobileScrollBody = page.locator(".proposals-list .scroll-body").first();

    await expect(mobileFrame).toBeVisible();
    await expect(mobileScrollBody).toBeVisible();

    await mobileFrame.hover();
    await page.mouse.wheel(0, 1200);

    await expect
      .poll(async () => {
        return mobileFrame.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    await expect
      .poll(async () => {
        return mobileScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBe(0);

    await page.setViewportSize({ width: 1365, height: 560 });
    await page.goto("/proposals");

    const desktopFrame = page.locator(".page-frame");
    const desktopPanel = page.locator(".proposals-list .panel").first();
    const desktopScrollBody = desktopPanel.locator(".scroll-body").first();
    const summaryPanel = page.locator(".proposals-summary .panel").first();

    await expect(desktopPanel).toBeVisible();
    await expect(summaryPanel).toBeVisible();
    await expect(desktopScrollBody).toBeVisible();

    const [listBox, summaryBox] = await Promise.all([desktopPanel.boundingBox(), summaryPanel.boundingBox()]);
    expect(listBox).not.toBeNull();
    expect(summaryBox).not.toBeNull();
    expect((summaryBox?.x ?? 0) < (listBox?.x ?? 0)).toBe(true);
    expect((listBox?.width ?? 0) > (summaryBox?.width ?? 0)).toBe(true);

    const ratio = (listBox?.width ?? 1) / Math.max(summaryBox?.width ?? 1, 1);
    expect(ratio).toBeGreaterThan(1.35);
    expect(ratio).toBeLessThan(1.65);

    await page.locator(".proposals-list .p-row").first().click();

    const overflows = await desktopScrollBody.evaluate((node) => node.scrollHeight > node.clientHeight);
    expect(overflows).toBe(true);

    await desktopScrollBody.hover();
    await page.mouse.wheel(0, 1200);

    await expect
      .poll(async () => {
        return desktopScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    await expect
      .poll(async () => {
        return desktopFrame.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });

  test("keeps desktop proposal scroll position when toggling a lower row", async ({ page }) => {
    await page.setViewportSize({ width: 1365, height: 560 });
    await page.goto("/proposals");

    const desktopScrollBody = page.locator(".proposals-list .scroll-body").first();

    await expect(desktopScrollBody).toBeVisible();

    await desktopScrollBody.hover();
    await page.mouse.wheel(0, 1400);

    await expect
      .poll(async () => {
        return desktopScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    const targetRow = page.locator(".proposals-list .p-row").nth(8);
    await targetRow.click();

    await expect(page).toHaveURL(/\/proposals\?hash=/);

    await expect
      .poll(async () => {
        return desktopScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);
  });

  test("keeps mobile page scroll position when toggling a lower row", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/proposals");

    const pageFrame = page.locator(".page-frame").first();
    await expect(pageFrame).toBeVisible();

    await pageFrame.hover();
    await page.mouse.wheel(0, 1200);

    await expect
      .poll(async () => {
        return pageFrame.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    const targetRow = page.locator(".proposals-list .p-row").nth(6);
    await targetRow.click();

    await expect(page).toHaveURL(/\/proposals\?hash=/);

    await expect
      .poll(async () => {
        return pageFrame.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);
  });

  test("reveals a direct hash-targeted proposal row without losing interactivity", async ({ page }) => {
    await page.setViewportSize({ width: 1365, height: 560 });
    await page.goto("/proposals");

    const targetRow = page.locator(".proposals-list .p-row").nth(8);
    const targetHash = await targetRow.getAttribute("data-proposal-hash");
    expect(targetHash).not.toBeNull();

    await page.goto(`/proposals?hash=${targetHash ?? ""}`);

    const targetedRow = page.locator(`.proposals-list .p-row[data-proposal-hash="${targetHash ?? ""}"]`).first();
    await expect(targetedRow).toBeVisible();
    await expect(targetedRow).toHaveClass(/p-row-expanded/);

    await page.locator(".proposals-list .p-row").nth(2).click();
    await expect(page).toHaveURL(/\/proposals\?hash=/);
  });

  test("keeps the proposal donut contained on desktop widths", async ({ page }) => {
    for (const viewport of [
      { width: 1365, height: 768 },
      { width: 2560, height: 1080 },
    ]) {
      await page.setViewportSize(viewport);
      await page.goto("/proposals");

      const summaryPanel = page.locator(".proposals-summary .panel").first();
      const summaryVisual = summaryPanel.locator(".summary-visual").first();
      const summaryInfo = summaryPanel.locator(".summary-info").first();
      const donut = summaryPanel.locator(".donut-shell").first();

      await expect(summaryPanel).toBeVisible();
      await expect(summaryVisual).toBeVisible();
      await expect(summaryInfo).toBeVisible();
      await expect(donut).toBeVisible();

      const [panelBox, visualBox, infoBox, donutBox] = await Promise.all([
        summaryPanel.boundingBox(),
        summaryVisual.boundingBox(),
        summaryInfo.boundingBox(),
        donut.boundingBox(),
      ]);

      expect(panelBox).not.toBeNull();
      expect(visualBox).not.toBeNull();
      expect(infoBox).not.toBeNull();
      expect(donutBox).not.toBeNull();

      expect((donutBox?.x ?? 0) >= (visualBox?.x ?? 0) - 1).toBe(true);
      expect((donutBox?.y ?? 0) >= (visualBox?.y ?? 0) - 1).toBe(true);
      expect((donutBox?.x ?? 0) + (donutBox?.width ?? 0) <= (visualBox?.x ?? 0) + (visualBox?.width ?? 0) + 1).toBe(
        true
      );
      expect((donutBox?.y ?? 0) + (donutBox?.height ?? 0) <= (visualBox?.y ?? 0) + (visualBox?.height ?? 0) + 1).toBe(
        true
      );
      expect((donutBox?.y ?? 0) + (donutBox?.height ?? 0) < (infoBox?.y ?? 0)).toBe(true);
      expect((panelBox?.x ?? 0) <= (donutBox?.x ?? 0)).toBe(true);
      expect((donutBox?.x ?? 0) + (donutBox?.width ?? 0) <= (panelBox?.x ?? 0) + (panelBox?.width ?? 0) + 1).toBe(true);
    }
  });

  test("sizes the proposal donut monotonically across width changes", async ({ page }) => {
    const getDonutWidth = async (): Promise<number> => {
      const box = await page.locator(".proposals-summary .donut-shell").first().boundingBox();
      expect(box).not.toBeNull();
      return box?.width ?? 0;
    };

    await page.setViewportSize({ width: 620, height: 812 });
    await page.goto("/proposals");
    const narrowMobile = await getDonutWidth();

    await page.setViewportSize({ width: 700, height: 812 });
    await page.goto("/proposals");
    const wideMobile = await getDonutWidth();

    expect(wideMobile).toBeGreaterThanOrEqual(narrowMobile);

    await page.setViewportSize({ width: 1520, height: 900 });
    await page.goto("/proposals");
    const desktop16 = await getDonutWidth();

    await page.setViewportSize({ width: 1800, height: 900 });
    await page.goto("/proposals");
    const desktopWide = await getDonutWidth();

    expect(desktopWide).toBeGreaterThanOrEqual(desktop16);
  });
});
