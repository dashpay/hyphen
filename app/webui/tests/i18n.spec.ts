import { readFileSync } from "node:fs";
import JSON5 from "json5";
import { expect, test, type Page } from "playwright/test";

const ruMessages = JSON5.parse<{ readonly strings: Record<string, string> }>(
  readFileSync(new URL("../../../contrib/i18n/ru.json5", import.meta.url), "utf8")
).strings;
const i18nMeta = JSON5.parse<{ readonly langs: Record<string, { readonly name: string; readonly src: string }> }>(
  readFileSync(new URL("../../../contrib/i18n/meta.json5", import.meta.url), "utf8")
);
const russianNativeName = i18nMeta.langs["ru-RU"]?.name ?? "Русский";

function ru(key: string): string {
  return ruMessages[key] ?? key;
}

function fillTemplate(template: string, ...values: readonly string[]): string {
  let nextValueIndex = 0;

  return template.replaceAll("{}", (): string => values[nextValueIndex++] ?? "");
}

async function selectLocale(page: Page, currentLabel: string, nextLabel: string): Promise<void> {
  await page.getByRole("button", { name: currentLabel }).click();
  await page.getByRole("button", { name: nextLabel }).click();
}

test.describe("locale switching", () => {
  test.describe.configure({ mode: "serial" });

  test("switches to russian and persists across routes", async ({ page }) => {
    const ruSearchPlaceholder = fillTemplate(
      ru("Search {}"),
      [ru("block"), ru("transaction"), ru("address"), ru("validator"), ru("proposal")].join(", ")
    );

    await page.goto("/");
    await page.evaluate(() => {
      globalThis.localStorage.removeItem("hyphen-webui-locale");
    });
    await page.reload();

    await expect(page.getByRole("heading", { name: "Hyphen" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();

    await selectLocale(page, "English", russianNativeName);

    await expect(page.getByRole("button", { name: russianNativeName })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Overview") })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Blocks") })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Transactions"), exact: true })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Validators") })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Proposals") })).toBeVisible();
    await expect(page.getByPlaceholder(ruSearchPlaceholder)).toBeVisible();
    await expect(page.getByText(ru("Block Height"))).toBeVisible();
    await expect(page.getByText(ru("Hash Rate"))).toBeVisible();
    await expect(page.getByText(ru("Processed Transactions"))).toBeVisible();
    await expect(page.getByText(ru("Blockchain Size"))).toBeVisible();
    await expect(page.getByText(fillTemplate(ru("Powered by Hyphen v{}."), "0.1.0"))).toBeVisible();

    await page.getByRole("link", { name: ru("Transactions"), exact: true }).click();
    await expect(page.getByText(ru("Transactions"))).toBeVisible();

    const pendingTag = page.locator(".col-type .hy-tag").first();
    if ((await pendingTag.count()) > 0) {
      const pendingLabel = (await pendingTag.textContent())?.trim() ?? "";
      expect(
        new Set([
          ru("Payment"),
          ru("Reward"),
          ru("Data"),
          ru("Lock"),
          ru("Unlock"),
          ru("Register"),
          ru("Update"),
          ru("Revoke"),
          ru("Commit"),
          ru("ForkSig"),
        ]).has(pendingLabel)
      ).toBe(true);
    }

    await page.getByRole("link", { name: ru("Balances") }).click();
    await expect(page.getByRole("link", { name: ru("Balances") })).toBeVisible();

    await page.reload();
    await expect(page.getByRole("button", { name: russianNativeName })).toBeVisible();
    await expect(page.getByText(ru("Balances"))).toBeVisible();

    await selectLocale(page, russianNativeName, "English");

    await expect(page.getByRole("button", { name: "English" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Transactions", exact: true })).toBeVisible();
    await expect(page.getByPlaceholder(/Search/)).toBeVisible();
  });

  test("applies the stored locale on first paint", async ({ page }) => {
    await page.addInitScript(() => {
      globalThis.localStorage.setItem("hyphen-webui-locale", "ru");
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    await expect(page.getByRole("button", { name: russianNativeName })).toBeVisible();
    await expect(page.getByRole("link", { name: ru("Overview") })).toBeVisible();
    await expect(page.getByRole("link", { name: "Overview" })).toHaveCount(0);
    await expect(page.locator("html")).toHaveAttribute("lang", "ru-RU");
  });

  test("keeps russian labels through client-side navigation without english fallback", async ({ page }) => {
    await page.addInitScript(() => {
      globalThis.localStorage.setItem("hyphen-webui-locale", "ru");
    });

    await page.goto("/");
    await expect(page.getByText(ru("Transactions"))).toBeVisible();
    await expect(page.getByText("Transactions")).toHaveCount(0);

    await page.getByRole("link", { name: ru("Transactions"), exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", "ru-RU");
    await expect(page.getByRole("heading", { name: ru("Transactions") })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Transactions" })).toHaveCount(0);

    await page.getByRole("link", { name: ru("Blocks") }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", "ru-RU");
    await expect(page.getByRole("heading", { name: ru("Blocks") })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Blocks" })).toHaveCount(0);
  });
});
