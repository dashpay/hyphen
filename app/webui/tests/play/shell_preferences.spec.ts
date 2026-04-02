import { expect, test } from "playwright/test";

test.describe("shell preferences and light-mode search", () => {
  test("persists light mode across reloads", async ({ page }) => {
    await page.goto("/");

    const themeToggle = page.getByRole("switch").first();
    await themeToggle.click();
    await expect(page.locator("html")).toHaveClass(/theme-light/);
    await expect(themeToggle).toHaveAttribute("aria-checked", "true");

    await page.reload();
    await expect(page.locator("html")).toHaveClass(/theme-light/);
    await expect(page.getByRole("switch").first()).toHaveAttribute("aria-checked", "true");
  });

  test("persists sidebar collapse across reloads", async ({ page }) => {
    await page.goto("/");

    await page.getByRole("button", { name: "Collapse sidebar" }).click();
    await expect(page.getByRole("button", { name: "Expand sidebar" })).toBeVisible();

    await page.reload();
    await expect(page.getByRole("button", { name: "Expand sidebar" })).toBeVisible();
  });

  test("keeps search suggestions usable in light mode", async ({ page }) => {
    await page.addInitScript(() => {
      globalThis.localStorage.setItem("hyphen-webui-theme-light", "1");
    });

    await page.goto("/");
    await expect(page.locator("html")).toHaveClass(/theme-light/);

    const search = page.getByRole("searchbox").first();
    await search.fill("000");

    const dropdown = page.locator(".hy-search-results");
    await expect(dropdown).toBeVisible();

    const firstOption = dropdown.getByRole("option").first();
    await expect(firstOption).toBeVisible();
    await firstOption.click();

    await expect(page).toHaveURL(/\/(block|transaction|address|validator|proposals)\?/);
  });

  test("expands dock search while hiding locale and unit controls", async ({ page }) => {
    await page.setViewportSize({ width: 768, height: 900 });
    await page.goto("/");

    const searchBox = page.locator(".hy-search-box").first();
    const searchInput = page.getByRole("searchbox").first();
    const prefs = page.locator(".hy-shell-header-prefs").first();
    const localeTrigger = prefs.getByRole("button", { name: /English/ }).first();
    const unitTrigger = prefs.locator('[aria-expanded="false"]').nth(1);

    const collapsedBox = await searchBox.boundingBox();
    expect(collapsedBox).not.toBeNull();

    await expect(localeTrigger).toBeVisible();
    await expect(unitTrigger).toBeVisible();

    await searchInput.click();
    await searchInput.fill("000");

    const expandedBox = await searchBox.boundingBox();
    const dropdownBox = await page.locator(".hy-search-results").first().boundingBox();

    expect(expandedBox).not.toBeNull();
    expect(dropdownBox).not.toBeNull();
    expect(expandedBox?.width ?? 0).toBeGreaterThan(collapsedBox?.width ?? 0);
    expect(Math.abs((dropdownBox?.width ?? 0) - (expandedBox?.width ?? 0)) <= 2).toBe(true);

    await expect(prefs).toBeHidden();
  });

  test("keeps the unit dropdown inside a 375x667 mobile viewport", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/");

    const unitTrigger = page.locator(".hy-shell-header-prefs .header-dropdown").nth(1).getByRole("button").first();
    await expect(unitTrigger).toBeVisible();
    await unitTrigger.dispatchEvent("click");

    const menu = page.locator(".dropdown-menu").first();
    await expect(menu).toBeVisible();

    const menuBox = await menu.boundingBox();
    expect(menuBox).not.toBeNull();
    expect((menuBox?.x ?? 0) >= 0).toBe(true);
    expect((menuBox?.x ?? 0) + (menuBox?.width ?? 0) <= 375).toBe(true);
  });
});
