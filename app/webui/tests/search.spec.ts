import { expect, test } from "playwright/test";

test.describe("search autocomplete", () => {
  test("shows block prefix suggestions and routes to the highlighted result on enter", async ({ page }) => {
    await page.goto("/");

    const search = page.getByRole("searchbox").first();
    await search.fill("000000000000");

    const suggestions = page.getByRole("listbox", { name: "Search suggestions" });
    await expect(suggestions).toBeVisible();

    const options = suggestions.getByRole("option");
    await expect(options.first()).toHaveAttribute("aria-selected", "true");
    await expect(suggestions.getByText("Block").first()).toBeVisible();

    await search.press("Enter");

    await expect(page).toHaveURL(/\/block\?hash=/);
  });

  test("shows the first ten matching blocks for partial zero-prefixed hashes", async ({ page }) => {
    await page.goto("/");

    const search = page.getByRole("searchbox").first();
    await search.fill("000");

    const suggestions = page.getByRole("listbox", { name: "Search suggestions" });
    await expect(suggestions).toBeVisible();

    const options = suggestions.getByRole("option");
    await expect(options).toHaveCount(10);
    await expect(suggestions.getByText("canonical")).toHaveCount(0);

    const blockTags = suggestions.locator(".hy-tag");
    await expect(blockTags).toHaveCount(10);
    await expect(blockTags.first()).toHaveText("Block");
  });
});
