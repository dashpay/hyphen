import { expect, test, type APIRequestContext, type Locator, type Page } from "playwright/test";

interface ApiPage<T> {
  readonly value: readonly T[];
}

interface ApiProposalDetail {
  readonly name: string;
}

interface ApiBlockSummary {
  readonly hash: string;
}

async function readJson<T>(request: APIRequestContext, path: string): Promise<T> {
  const response = await request.get(`http://127.0.0.1:3080${path}`);
  expect(response.ok()).toBe(true);
  return (await response.json()) as T;
}

async function activateProposalSearch(page: Page): Promise<void> {
  const proposals = await readJson<ApiPage<ApiProposalDetail>>(page.request, "/api/v1/proposals?limit=1");
  const [proposal] = proposals.value;
  test.skip(proposal?.name == null || proposal.name.trim().length === 0, "no searchable proposal discovered");
  if (proposal == null) {
    return;
  }

  await page.goto("/");

  const search = page.getByRole("searchbox").first();
  await search.fill(proposal.name);
  await search.press("Enter");

  await expect(page).toHaveURL(/\/proposals\?hash=/);
}

async function expectNoHorizontalOverlap(left: Locator, right: Locator): Promise<void> {
  const [leftBox, rightBox] = await Promise.all([left.boundingBox(), right.boundingBox()]);

  expect(leftBox).not.toBeNull();
  expect(rightBox).not.toBeNull();
  expect((leftBox?.x ?? 0) + (leftBox?.width ?? 0) <= (rightBox?.x ?? 0)).toBe(true);
}

test.describe("search-driven navigation layout stability", () => {
  test("keeps validators table aligned after proposal search navigation", async ({ page }) => {
    await activateProposalSearch(page);
    await page.getByRole("link", { name: "Validators" }).click();
    await expect(page).toHaveURL("/validators");

    const firstRow = page.locator(".table-row").first();
    test.skip((await firstRow.count()) === 0, "no validator rows available in the current dataset");
    await expect(firstRow).toBeVisible({ timeout: 15000 });

    await expectNoHorizontalOverlap(firstRow.locator(".col-protx").first(), firstRow.locator(".col-status").first());
  });

  test("keeps block transaction rows readable after proposal search navigation", async ({ page }) => {
    const blocks = await readJson<ApiPage<ApiBlockSummary>>(page.request, "/api/v1/blocks?limit=1");
    const [block] = blocks.value;
    test.skip(block?.hash == null, "no block discovered");
    if (block == null) {
      return;
    }

    await activateProposalSearch(page);
    await page.goto(`/block?hash=${block.hash}`);

    const firstRow = page.locator(".tx-panel .tx-row").first();
    test.skip((await firstRow.count()) === 0, "no block transaction rows available in the current dataset");
    await expect(firstRow).toBeVisible({ timeout: 15000 });

    await expectNoHorizontalOverlap(
      firstRow.locator(".detail-row-top .hy-compact-row-link").first(),
      firstRow.locator(".hy-compact-row-side").first()
    );
  });
});
