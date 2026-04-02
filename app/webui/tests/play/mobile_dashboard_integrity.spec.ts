import { expect, test } from "playwright/test";

test.describe("mobile dashboard integrity", () => {
  test("uses Huninn for chinese locale", async ({ page }) => {
    await page.addInitScript(() => {
      globalThis.localStorage.setItem("hyphen-webui-locale", "zh");
    });

    await page.goto("/");

    const fontFamily = await page.evaluate((): string => {
      return globalThis.getComputedStyle(globalThis.document.body).fontFamily;
    });

    expect(fontFamily.includes("Huninn")).toBe(true);
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  });

  test("keeps the network dashboard scrollable on mobile and resets scroll on route return", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/network");

    const frame = page.locator(".page-frame");
    await expect(frame).toBeVisible();

    const hasOverflow = await frame.evaluate((node) => node.scrollHeight > node.clientHeight);
    if (hasOverflow) {
      await frame.hover();
      await page.mouse.wheel(0, 1200);

      await expect
        .poll(async () => {
          return frame.evaluate((node) => node.scrollTop);
        })
        .toBeGreaterThan(0);
    }

    await page.goto("/validators");
    await page.goto("/network");

    await expect
      .poll(async () => {
        return frame.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });

  test("keeps the balances donut within its panel after route switches", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/validators");
    await page.goto("/balances");

    const panel = page.locator(".concentration-panel .panel").first();
    const donut = page.locator(".concentration-panel .donut").first();

    await expect(panel).toBeVisible();
    await expect(donut).toBeVisible();

    const [panelBox, donutBox] = await Promise.all([panel.boundingBox(), donut.boundingBox()]);

    expect(panelBox).not.toBeNull();
    expect(donutBox).not.toBeNull();
    expect((donutBox?.x ?? 0) >= (panelBox?.x ?? 0) - 1).toBe(true);
    expect((donutBox?.y ?? 0) >= (panelBox?.y ?? 0) - 1).toBe(true);
    expect((donutBox?.x ?? 0) + (donutBox?.width ?? 0) <= (panelBox?.x ?? 0) + (panelBox?.width ?? 0) + 1).toBe(true);
    expect((donutBox?.y ?? 0) + (donutBox?.height ?? 0) <= (panelBox?.y ?? 0) + (panelBox?.height ?? 0) + 1).toBe(true);
  });

  test("keeps concentration fixed above balances on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/balances");

    const frame = page.locator(".page-frame");
    const concentrationPanel = page.locator(".balances-summary .concentration-panel .panel").first();
    const balancesPanel = page.locator(".balances-list .balances-table .panel").first();
    const balancesScrollBody = balancesPanel.locator(".scroll-body").first();
    const donutCenter = concentrationPanel.locator(".donut-center").first();
    const concentrationTable = concentrationPanel.locator(".concentration-table").first();

    await expect(frame).toBeVisible();
    await expect(concentrationPanel).toBeVisible();
    await expect(balancesPanel).toBeVisible();
    await expect(balancesScrollBody).toBeVisible();
    await expect(donutCenter).toBeVisible();
    await expect(concentrationTable).toBeVisible();
    await expect(concentrationPanel.locator(".scroll-body")).toHaveCount(0);

    const [concentrationBox, balancesBox, donutBox, tableBox] = await Promise.all([
      concentrationPanel.boundingBox(),
      balancesPanel.boundingBox(),
      donutCenter.boundingBox(),
      concentrationTable.boundingBox(),
    ]);

    expect(concentrationBox).not.toBeNull();
    expect(balancesBox).not.toBeNull();
    expect(donutBox).not.toBeNull();
    expect(tableBox).not.toBeNull();
    expect((concentrationBox?.y ?? 0) < (balancesBox?.y ?? 0)).toBe(true);
    expect(Math.abs((donutBox?.y ?? 0) - (tableBox?.y ?? 0)) < 24).toBe(true);
    expect((donutBox?.x ?? 0) < (tableBox?.x ?? 0)).toBe(true);

    await frame.hover();
    await page.mouse.wheel(0, 1200);

    await expect
      .poll(async () => {
        return frame.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    await expect
      .poll(async () => {
        return balancesScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });

  test("keeps balances wide and concentration narrow on desktop", async ({ page }) => {
    await page.setViewportSize({ width: 1365, height: 900 });
    await page.goto("/balances");

    const frame = page.locator(".page-frame");
    const concentrationPanel = page.locator(".balances-summary .concentration-panel .panel").first();
    const balancesPanel = page.locator(".balances-list .balances-table .panel").first();
    const balancesScrollBody = balancesPanel.locator(".scroll-body").first();

    await expect(concentrationPanel).toBeVisible();
    await expect(balancesPanel).toBeVisible();
    await expect(balancesScrollBody).toBeVisible();

    const [concentrationBox, balancesBox] = await Promise.all([
      concentrationPanel.boundingBox(),
      balancesPanel.boundingBox(),
    ]);

    expect(concentrationBox).not.toBeNull();
    expect(balancesBox).not.toBeNull();
    expect((balancesBox?.x ?? 0) < (concentrationBox?.x ?? 0)).toBe(true);
    expect((balancesBox?.width ?? 0) > (concentrationBox?.width ?? 0)).toBe(true);
    expect(
      Math.abs(
        (balancesBox?.y ?? 0) +
          (balancesBox?.height ?? 0) -
          ((concentrationBox?.y ?? 0) + (concentrationBox?.height ?? 0))
      ) < 4
    ).toBe(true);

    const balancesBodyOverflows = await balancesScrollBody.evaluate((node) => node.scrollHeight > node.clientHeight);
    expect(balancesBodyOverflows).toBe(true);

    await balancesScrollBody.hover();
    await page.mouse.wheel(0, 1200);

    await expect
      .poll(async () => {
        return balancesScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    await expect
      .poll(async () => {
        return frame.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });

  test("keeps desktop balances split before the mobile dock breakpoint", async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 900 });
    await page.goto("/balances");

    const concentrationPanel = page.locator(".balances-summary .concentration-panel .panel").first();
    const balancesPanel = page.locator(".balances-list .balances-table .panel").first();

    await expect(concentrationPanel).toBeVisible();
    await expect(balancesPanel).toBeVisible();

    const [concentrationBox, balancesBox] = await Promise.all([
      concentrationPanel.boundingBox(),
      balancesPanel.boundingBox(),
    ]);

    expect(concentrationBox).not.toBeNull();
    expect(balancesBox).not.toBeNull();
    expect((balancesBox?.x ?? 0) < (concentrationBox?.x ?? 0)).toBe(true);
    expect(Math.abs((balancesBox?.y ?? 0) - (concentrationBox?.y ?? 0)) < 24).toBe(true);
  });

  test("keeps latest blocks reachable after returning from network on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/network");
    await page.goto("/");

    const frame = page.locator(".page-frame");
    await expect(frame).toBeVisible();

    await frame.evaluate((node) => {
      node.scrollTop = node.scrollHeight;
    });

    await expect(page.getByText("Latest Blocks", { exact: true })).toBeVisible();
  });
});
