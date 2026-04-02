import { readFileSync } from "node:fs";
import JSON5 from "json5";
import { expect, test } from "playwright/test";

const ruMessages = JSON5.parse<{ readonly strings: Record<string, string> }>(
  readFileSync(new URL("../../../../contrib/i18n/ru.json5", import.meta.url), "utf8")
).strings;
const i18nMeta = JSON5.parse<{ readonly langs: Record<string, { readonly name: string; readonly src: string }> }>(
  readFileSync(new URL("../../../../contrib/i18n/meta.json5", import.meta.url), "utf8")
);
const russianNativeName = i18nMeta.langs["ru-RU"]?.name ?? "Русский";

function ru(key: string): string {
  return ruMessages[key] ?? key;
}

test.describe("locale persistence and no-flash", () => {
  test("applies russian on first paint without english fallback", async ({ page }) => {
    await page.addInitScript(() => {
      globalThis.localStorage.setItem("hyphen-webui-locale", "ru");
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    await expect(page.getByRole("button", { name: russianNativeName })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Overview") })).toBeVisible();
    await expect(page.getByRole("link", { name: "Overview" })).toHaveCount(0);
    await expect(page.locator("html")).toHaveAttribute("lang", "ru-RU");
  });

  test("keeps russian labels through client-side navigation", async ({ page }) => {
    await page.addInitScript(() => {
      globalThis.localStorage.setItem("hyphen-webui-locale", "ru");
    });

    await page.goto("/");
    await page.getByRole("link", { name: ru("Transactions"), exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", "ru-RU");
    await expect(page.getByRole("link", { name: ru("Blocks") })).toBeVisible({ timeout: 15000 });
    await expect(page.getByRole("link", { name: "Blocks" })).toHaveCount(0);

    await page.getByRole("link", { name: ru("Blocks") }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", "ru-RU");
    await expect(page.getByRole("heading", { name: ru("Blocks") })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Blocks" })).toHaveCount(0);
  });
});
