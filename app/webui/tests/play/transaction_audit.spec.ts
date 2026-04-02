import { expect, test, type APIRequestContext, type Page } from "playwright/test";

interface ApiPage<T> {
  readonly value: readonly T[];
}

interface ApiBlockSummary {
  readonly hash: string;
}

interface ApiTxSummary {
  readonly txType: string;
  readonly txid: string;
}

interface ApiValidatorSummary {
  readonly proTxHash: string;
}

interface ApiValidatorDetail {
  readonly history: readonly {
    readonly txType: string;
    readonly txid: string;
  }[];
}

interface ApiTxDetail {
  readonly payload: unknown;
}

interface TransactionExpectation {
  readonly label: string;
  readonly payloadField?: string;
  readonly showsPayload: boolean;
}

const transactionExpectations: Readonly<Record<string, TransactionExpectation>> = {
  asset_lock: { label: "Lock", payloadField: "Version", showsPayload: true },
  asset_unlock: { label: "Unlock", payloadField: "Version", showsPayload: true },
  coinbase: { label: "Reward", payloadField: "Height", showsPayload: true },
  data: { label: "Data", payloadField: "Hex", showsPayload: true },
  mn_hf_signal: { label: "ForkSig", payloadField: "Version", showsPayload: true },
  mnhf: { label: "ForkSig", payloadField: "Version", showsPayload: true },
  normal: { label: "Payment", showsPayload: false },
  payment: { label: "Payment", showsPayload: false },
  provider_register: { label: "Register", payloadField: "Version", showsPayload: true },
  provider_update_registrar: { label: "Update", payloadField: "Version", showsPayload: true },
  provider_update_revoke: { label: "Revoke", payloadField: "Version", showsPayload: true },
  provider_update_service: { label: "Update", payloadField: "Version", showsPayload: true },
  quorum_commitment: { label: "Commit", payloadField: "Version", showsPayload: true },
  standard: { label: "Payment", showsPayload: false },
};

async function readJson<T>(request: APIRequestContext, path: string): Promise<T> {
  const response = await request.get(`http://127.0.0.1:3080${path}`);
  expect(response.ok()).toBe(true);
  return (await response.json()) as T;
}

function readPayloadKeys(payload: unknown): readonly string[] {
  if (payload == null || typeof payload !== "object" || Array.isArray(payload)) {
    return [];
  }

  return Object.keys(payload as Record<string, unknown>);
}

async function discoverTransactionKinds(request: APIRequestContext): Promise<ReadonlyMap<string, string>> {
  const discovered = new Map<string, string>();

  const latestTransactions = await readJson<ApiPage<ApiTxSummary>>(request, "/api/v1/txs?limit=100");
  for (const transaction of latestTransactions.value) {
    if (!(transaction.txType in transactionExpectations) || discovered.has(transaction.txType)) {
      continue;
    }
    discovered.set(transaction.txType, transaction.txid);
  }

  const latestBlocks = await readJson<ApiPage<ApiBlockSummary>>(request, "/api/v1/blocks?limit=10");
  for (const block of latestBlocks.value) {
    const blockTransactions = await readJson<ApiPage<ApiTxSummary>>(
      request,
      `/api/v1/block/${block.hash}/txs?limit=100`
    );

    for (const transaction of blockTransactions.value) {
      if (!(transaction.txType in transactionExpectations) || discovered.has(transaction.txType)) {
        continue;
      }
      discovered.set(transaction.txType, transaction.txid);
    }
  }

  const validators = await readJson<ApiPage<ApiValidatorSummary>>(request, "/api/v1/validators?limit=10");
  for (const validator of validators.value) {
    const detail = await readJson<ApiValidatorDetail>(request, `/api/v1/validator/${validator.proTxHash}`);

    for (const entry of detail.history) {
      if (!(entry.txType in transactionExpectations) || discovered.has(entry.txType)) {
        continue;
      }
      discovered.set(entry.txType, entry.txid);
    }
  }

  return discovered;
}

async function assertTransactionRendering(
  page: Page,
  request: APIRequestContext,
  txType: string,
  txid: string
): Promise<void> {
  const expectation = transactionExpectations[txType];
  if (expectation == null) {
    throw new Error(`missing transaction expectation for ${txType}`);
  }

  const detail = await readJson<ApiTxDetail>(request, `/api/v1/tx/${txid}`);
  const rawPayloadKeys = readPayloadKeys(detail.payload);

  await page.goto(`/transaction?hash=${txid}`);
  await expect(page.getByRole("heading", { name: "Transaction" })).toBeVisible();
  await expect(page.locator(".hy-tag").first()).toHaveText(expectation.label);

  const payloadTable = page.locator(".hy-detail-payload-table");

  if (expectation.showsPayload) {
    await expect(page.getByText("Payload", { exact: true })).toBeVisible();
    if (expectation.payloadField != null) {
      await expect(payloadTable.getByText(expectation.payloadField, { exact: true })).toBeVisible();
    }
  } else {
    await expect(page.getByText("Payload", { exact: true })).toHaveCount(0);
  }

  for (const rawKey of rawPayloadKeys) {
    await expect(payloadTable.getByText(rawKey, { exact: true })).toHaveCount(0);
  }
}

test.describe("transaction rendering audit", () => {
  test("renders discovered transaction kinds without raw payload labels", async ({ page }) => {
    const discovered = await discoverTransactionKinds(page.request);
    test.skip(discovered.size === 0, "no auditable transaction kinds discovered in the current dataset");

    for (const [txType, txid] of discovered) {
      await assertTransactionRendering(page, page.request, txType, txid);
    }
  });
});
