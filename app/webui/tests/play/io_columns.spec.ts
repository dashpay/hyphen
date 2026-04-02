import { expect, test, type APIRequestContext, type Locator } from "playwright/test";

interface ApiPage<T> {
  readonly value: readonly T[];
}

interface ApiBalanceEntry {
  readonly address: string;
  readonly txCount: number;
}

interface ApiAddressTxEntry {
  readonly txid: string;
}

interface ApiBlockSummary {
  readonly hash: string;
}

async function readJson<T>(request: APIRequestContext, path: string): Promise<T> {
  const response = await request.get(`http://127.0.0.1:3080${path}`);
  expect(response.ok()).toBe(true);
  return (await response.json()) as T;
}

async function findExpandableAddress(request: APIRequestContext): Promise<string | null> {
  const balances = await readJson<ApiPage<ApiBalanceEntry>>(request, "/api/v1/balances?limit=20");

  for (const entry of balances.value) {
    if (entry.txCount <= 0) {
      continue;
    }

    const txs = await readJson<ApiPage<ApiAddressTxEntry>>(request, `/api/v1/address/${entry.address}/txs?limit=1`);
    if (txs.value.length > 0) {
      return entry.address;
    }
  }

  return null;
}

async function expectSideBySidePanels(ioColumns: Locator): Promise<void> {
  const panels = ioColumns.locator(".io-panel");
  await expect(panels).toHaveCount(2);

  const [fromPanel, toPanel] = [panels.nth(0), panels.nth(1)];
  await expect(fromPanel).toBeVisible();
  await expect(toPanel).toBeVisible();

  const [fromBox, toBox] = await Promise.all([fromPanel.boundingBox(), toPanel.boundingBox()]);
  expect(fromBox).not.toBeNull();
  expect(toBox).not.toBeNull();
  expect(Math.abs((fromBox?.y ?? 0) - (toBox?.y ?? 0)) < 8).toBe(true);
  expect((fromBox?.x ?? 0) + (fromBox?.width ?? 0) <= (toBox?.x ?? 0)).toBe(true);
}

async function expectStackedAmount(panel: Locator): Promise<void> {
  const firstItem = panel.locator(".io-item").first();
  const label = firstItem.locator(".io-body > :first-child").first();
  const amount = firstItem.locator(".io-body > :last-child").first();

  await expect(label).toBeVisible();
  await expect(amount).toBeVisible();

  const [labelBox, amountBox] = await Promise.all([label.boundingBox(), amount.boundingBox()]);
  expect(labelBox).not.toBeNull();
  expect(amountBox).not.toBeNull();
  expect((amountBox?.y ?? 0) > (labelBox?.y ?? 0)).toBe(true);

  const address = firstItem.locator(".io-addr").first();
  if ((await address.count()) > 0) {
    const addressMetrics = await address.evaluate((node) => {
      const element = node as HTMLElement;
      const style = globalThis.getComputedStyle(element);
      return {
        overflow: style.overflow,
        textOverflow: style.textOverflow,
        whiteSpace: style.whiteSpace,
      };
    });

    expect(addressMetrics.overflow).toBe("hidden");
    expect(addressMetrics.textOverflow).toBe("ellipsis");
    expect(addressMetrics.whiteSpace).toBe("nowrap");
  }
}

test.describe("transaction io columns", () => {
  test("keeps a simple transaction detail in two columns", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto("/transaction?hash=7c4e54deaad96f44e4c2fb5ac5a74451231678cff89fc89f59334a9a09ad0df1");

    const ioColumns = page.locator(".hy-detail-io-columns").first();
    await expect(ioColumns).toBeVisible();
    await expectSideBySidePanels(ioColumns);
    await expectStackedAmount(ioColumns.locator(".io-panel").first());
  });

  test("keep address transaction expansion in two columns on mobile", async ({ page }) => {
    const address = await findExpandableAddress(page.request);
    test.skip(address == null, "no expandable address discovered");
    if (address == null) {
      return;
    }

    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto(`/address?id=${address}`);

    const firstRow = page.locator(".tx-panel .tx-row").first();
    await expect(firstRow).toBeVisible({ timeout: 15000 });
    await firstRow.click();

    const ioColumns = page.locator(".expanded-body .hy-detail-io-columns").first();
    await expect(ioColumns).toBeVisible();
    await expectSideBySidePanels(ioColumns);
    await expectStackedAmount(ioColumns.locator(".io-panel").first());
  });

  test("keep block transaction expansion in two columns on mobile", async ({ page }) => {
    const blocks = await readJson<ApiPage<ApiBlockSummary>>(page.request, "/api/v1/blocks?limit=1");
    const [block] = blocks.value;
    test.skip(block?.hash == null, "no block discovered");
    if (block == null) {
      return;
    }

    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto(`/block?hash=${block.hash}`);

    const firstRow = page.locator(".tx-panel .tx-row").first();
    await expect(firstRow).toBeVisible({ timeout: 15000 });
    await firstRow.click();

    const ioColumns = page.locator(".expanded-body .hy-detail-io-columns").first();
    await expect(ioColumns).toBeVisible();
    await expectSideBySidePanels(ioColumns);
    await expectStackedAmount(ioColumns.locator(".io-panel").nth(1));
  });
});
