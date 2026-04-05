import { expect, test } from "playwright/test";

test.describe("mobile dashboard integrity", () => {
  test("uses the intended overview mobile composition and page-owned list scroll", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/");

    const frame = page.locator(".page-frame");
    const mobileSummary = page.locator(".overview-mobile-summary").first();
    const mobileBlocks = page.locator(".overview-mobile-blocks").first();
    const mobileGraphs = page.locator(".overview-mobile-graphs").first();
    const blockHeightCard = mobileSummary.getByText("Block Height", { exact: true });
    const latestBlocksPanel = mobileBlocks.locator(".block-panel .panel").first();
    const latestBlocksHeading = latestBlocksPanel.getByText("Latest Blocks", { exact: true });
    const processedCard = mobileGraphs.getByText("Processed Transactions", { exact: true });
    const hashRateCard = mobileGraphs.getByText("Hash Rate", { exact: true });
    const pendingPanel = page.locator(".overview-pending .tx-panel .panel").first();
    const pendingHeading = pendingPanel.getByText("Pending Transactions", { exact: true });
    const latestBlocksScrollBody = latestBlocksPanel.locator(".scroll-body").first();
    const pendingScrollBody = pendingPanel.locator(".scroll-body").first();

    await expect(frame).toBeVisible();
    await expect(blockHeightCard).toBeVisible();
    await expect(latestBlocksHeading).toBeVisible();
    await expect(processedCard).toBeVisible();
    await expect(hashRateCard).toBeVisible();
    await expect(pendingHeading).toBeVisible();

    const [blockHeightBox, latestBlocksBox, processedBox, hashRateBox, pendingBox] = await Promise.all([
      blockHeightCard.boundingBox(),
      latestBlocksPanel.boundingBox(),
      processedCard.boundingBox(),
      hashRateCard.boundingBox(),
      pendingPanel.boundingBox(),
    ]);

    expect(blockHeightBox).not.toBeNull();
    expect(latestBlocksBox).not.toBeNull();
    expect(processedBox).not.toBeNull();
    expect(hashRateBox).not.toBeNull();
    expect(pendingBox).not.toBeNull();

    expect((latestBlocksBox?.y ?? 0) > (blockHeightBox?.y ?? 0)).toBe(true);
    expect(Math.abs((processedBox?.y ?? 0) - (hashRateBox?.y ?? 0)) < 24).toBe(true);
    expect((hashRateBox?.x ?? 0) > (processedBox?.x ?? 0)).toBe(true);
    expect((processedBox?.y ?? 0) > (latestBlocksBox?.y ?? 0)).toBe(true);
    expect((pendingBox?.y ?? 0) > (processedBox?.y ?? 0)).toBe(true);

    await frame.hover();
    await page.mouse.wheel(0, 1200);

    await expect
      .poll(async () => {
        return frame.evaluate((node) => node.scrollTop);
      })
      .toBeGreaterThan(0);

    await expect
      .poll(async () => {
        return latestBlocksScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBe(0);

    await expect
      .poll(async () => {
        return pendingScrollBody.evaluate((node) => node.scrollTop);
      })
      .toBe(0);
  });

  test("uses the intended network mobile composition", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/network");

    const versionsPanel = page.locator(".versions-panel .panel").first();
    const mobileCounters = page.locator(".mobile-network-counters").first();
    const utxoCard = mobileCounters.getByText("UTXO Set", { exact: true });
    const chainstateCard = mobileCounters.getByText("Chainstate", { exact: true });
    const nodeStatusPanel = page.locator(".donut-panel .panel").first();

    await expect(versionsPanel).toBeVisible();
    await expect(mobileCounters).toBeVisible();
    await expect(utxoCard).toBeVisible();
    await expect(chainstateCard).toBeVisible();
    await expect(nodeStatusPanel).toBeVisible();

    const [versionsBox, utxoBox, chainstateBox, nodeStatusBox] = await Promise.all([
      versionsPanel.boundingBox(),
      utxoCard.boundingBox(),
      chainstateCard.boundingBox(),
      nodeStatusPanel.boundingBox(),
    ]);

    expect(versionsBox).not.toBeNull();
    expect(utxoBox).not.toBeNull();
    expect(chainstateBox).not.toBeNull();
    expect(nodeStatusBox).not.toBeNull();

    expect((utxoBox?.y ?? 0) > (versionsBox?.y ?? 0)).toBe(true);
    expect(Math.abs((utxoBox?.y ?? 0) - (chainstateBox?.y ?? 0)) < 24).toBe(true);
    expect((chainstateBox?.x ?? 0) > (utxoBox?.x ?? 0)).toBe(true);
    expect((nodeStatusBox?.y ?? 0) > (utxoBox?.y ?? 0)).toBe(true);
  });

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
    const latestBlocksHeading = page.locator(".overview-mobile-blocks").getByText("Latest Blocks", { exact: true });
    await expect(frame).toBeVisible();

    await frame.evaluate((node) => {
      node.scrollTop = node.scrollHeight;
    });

    await expect(latestBlocksHeading).toBeVisible();
  });
});
