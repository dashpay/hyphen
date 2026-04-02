import { expect, test, type Page } from "playwright/test";

interface TotalPage {
  readonly totalCount?: number | null;
}

function formatCount(count: number): string {
  return new Intl.NumberFormat("en-US").format(count);
}

async function readRouteTotal(page: Page, routePath: string): Promise<number | null> {
  const responsePromise = page.waitForResponse(
    (response): boolean => response.url().includes(routePath) && response.request().method() === "GET"
  );

  const response = await responsePromise;
  expect(response.ok()).toBe(true);
  const body = (await response.json()) as TotalPage;
  return body.totalCount ?? null;
}

test.describe("browser list totals", () => {
  test("shows the balances total from the API", async ({ page }) => {
    const totalPromise = readRouteTotal(page, "/api/v1/balances?");
    await page.goto("/balances");
    const totalCount = await totalPromise;
    test.skip(totalCount == null, "balances totalCount unavailable");
    if (totalCount == null) {
      return;
    }
    await expect(page.getByText(`${formatCount(totalCount)} addresses`, { exact: true })).toBeVisible();
  });

  test("shows the validator total from the API", async ({ page }) => {
    const totalPromise = readRouteTotal(page, "/api/v1/validators?");
    await page.goto("/validators");
    const totalCount = await totalPromise;
    test.skip(totalCount == null, "validators totalCount unavailable");
    if (totalCount == null) {
      return;
    }
    await expect(page.getByText(new RegExp(`^${formatCount(totalCount)} validators? \\(`))).toBeVisible();
  });

  test("shows the proposal total from the API", async ({ page }) => {
    const totalPromise = readRouteTotal(page, "/api/v1/proposals?");
    await page.goto("/proposals");
    const totalCount = await totalPromise;
    test.skip(totalCount == null, "proposals totalCount unavailable");
    if (totalCount == null) {
      return;
    }
    await expect(page.getByText(new RegExp(`^${formatCount(totalCount)} proposals? \\(`))).toBeVisible();
  });
});
