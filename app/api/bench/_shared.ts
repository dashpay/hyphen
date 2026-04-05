import {
  ApCreateClient,
  type ApApiError,
  type ApBlockSummary,
  type ApClient,
  type ApResult,
  type ApTransactionSummary,
  type ApValidatorSummary,
} from "../src/index.js";

const benchUrl = process.env["HYPHEN_API_LIVE_URL"];

if (typeof benchUrl !== "string" || benchUrl.length === 0) {
  throw new Error("bench:api requires HYPHEN_API_LIVE_URL=<hyphen-api-base-url>");
}

export const benchClient: ApClient = ApCreateClient({ baseUrl: benchUrl });

export interface BenchDiscoveryContext {
  readonly address: string;
  readonly block: ApBlockSummary;
  readonly proposalHash: string | null;
  readonly searchQuery: string;
  readonly transaction: ApTransactionSummary;
  readonly validator: ApValidatorSummary;
}

let benchDiscovery: BenchDiscoveryContext | null = null;

export function unwrapBenchResult<TValue>(name: string, result: ApResult<TValue, ApApiError>): TValue {
  const value = result.value();

  if (value == null) {
    const error = result.errorOr(new Error(`${name} failed without an error payload`) as never);
    throw new Error(`${name} failed: ${error.kind} ${error.message}`);
  }

  return value;
}

async function discoverContext(): Promise<BenchDiscoveryContext> {
  const blockPage = unwrapBenchResult("blocks.list", await benchClient.blocks.list({ limit: 1 }));
  const block = blockPage.value[0];

  if (block == null) {
    throw new Error("blocks.list returned no rows");
  }

  const transactionPage = unwrapBenchResult("transactions.list", await benchClient.transactions.list({ limit: 1 }));
  const transaction = transactionPage.value[0];

  if (transaction == null) {
    throw new Error("transactions.list returned no rows");
  }

  const balancePage = unwrapBenchResult(
    "addresses.listBalances",
    await benchClient.addresses.listBalances({ limit: 1 })
  );
  const address = balancePage.value[0]?.address;

  if (address == null) {
    throw new Error("addresses.listBalances returned no rows");
  }

  const validatorPage = unwrapBenchResult("validators.list", await benchClient.validators.list({ limit: 1 }));
  const validator = validatorPage.value[0];

  if (validator == null) {
    throw new Error("validators.list returned no rows");
  }

  const proposalPage = unwrapBenchResult(
    "governanceSearch.listProposals",
    await benchClient.governanceSearch.listProposals({ limit: 1 })
  );
  const proposalHash = proposalPage.value[0]?.hash ?? null;

  return {
    address,
    block,
    proposalHash,
    searchQuery: transaction.txHash,
    transaction,
    validator,
  };
}

export async function getBenchDiscovery(): Promise<BenchDiscoveryContext> {
  if (benchDiscovery != null) {
    return benchDiscovery;
  }

  benchDiscovery = await discoverContext();
  return benchDiscovery;
}
