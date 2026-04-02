import { expect, test } from "playwright/test";

test.describe("shell navigation resilience", () => {
  test("sidebar stays responsive from a failed block detail route", async ({ page }) => {
    await page.goto("/block?hash=0000000000000000000000000000000000000000000000000000000000000000");
    await expect(page).toHaveURL(/\/block\?hash=/);

    await page.getByRole("link", { name: "Overview" }).click();
    await expect(page).toHaveURL("/");
    await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
  });

  test("sidebar stays responsive from a failed proposal route", async ({ page }) => {
    await page.goto("/proposals?hash=0000000000000000000000000000000000000000000000000000000000000000");
    await expect(page).toHaveURL(/\/proposals\?hash=/);

    await page.getByRole("link", { name: "Validators" }).click();
    await expect(page).toHaveURL("/validators");
    await expect(page.getByRole("link", { name: "Validators" })).toBeVisible();
  });
});
