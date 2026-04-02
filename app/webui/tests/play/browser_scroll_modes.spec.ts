import { expect, test, type Page } from "playwright/test";

async function expectDesktopListScroll(page: Page, href: string): Promise<void> {
  await page.setViewportSize({ width: 1365, height: 700 });
  await page.goto(href);

  const frame = page.locator(".page-frame");
  const scrollBody = page.locator(".scroll-body").first();

  await expect(frame).toBeVisible();
  await expect(scrollBody).toBeVisible();

  await scrollBody.hover();
  await page.mouse.wheel(0, 1200);

  await expect
    .poll(async () => {
      return scrollBody.evaluate((node) => node.scrollTop);
    })
    .toBeGreaterThan(0);

  await expect
    .poll(async () => {
      return frame.evaluate((node) => node.scrollTop);
    })
    .toBe(0);
}

async function expectDockPageScroll(page: Page, href: string): Promise<void> {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto(href);

  const frame = page.locator(".page-frame");
  const scrollBody = page.locator(".scroll-body").first();

  await expect(frame).toBeVisible();
  await expect(scrollBody).toBeVisible();

  await frame.hover();
  await page.mouse.wheel(0, 1200);

  await expect
    .poll(async () => {
      return frame.evaluate((node) => node.scrollTop);
    })
    .toBeGreaterThan(0);

  await expect
    .poll(async () => {
      return scrollBody.evaluate((node) => node.scrollTop);
    })
    .toBe(0);
}

test.describe("browser scroll modes", () => {
  test("keeps blocks on local scroll for desktop and page scroll for dock", async ({ page }) => {
    await expectDesktopListScroll(page, "/blocks");
    await expectDockPageScroll(page, "/blocks");
  });

  test("keeps validators on local scroll for desktop and page scroll for dock", async ({ page }) => {
    await expectDesktopListScroll(page, "/validators");
    await expectDockPageScroll(page, "/validators");
  });

  test("keeps transactions on local scroll for desktop and page scroll for dock", async ({ page }) => {
    await expectDesktopListScroll(page, "/transactions");
    await expectDockPageScroll(page, "/transactions");
  });
});
