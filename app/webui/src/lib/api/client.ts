import {
  ApCreateClient,
  type ApClient,
  type ApAddressSummary,
  type ApAddressTransactionSummary,
  type ApAddressUtxo,
  type ApApiError,
  type ApBalancePage,
  type ApBlockDetail,
  type ApBlockSummary,
  type ApMempoolTransactionSummary,
  type ApNetworkSnapshot,
  type ApProposalDetail,
  type ApProposalVote,
  type ApRawBlockResponse,
  type ApRawTxResponse,
  type ApResult,
  type ApSearchResult,
  type ApTransactionDetail,
  type ApTransactionSummary,
  type ApValidatorDetail,
  type ApValidatorHistoryEntry,
  type ApValidatorSummary,
} from "@repo/hyphen-api";

import type {
  ApiAddressSummary,
  ApiAddressTxListResponse,
  ApiBlockDetail,
  ApiBlockListResponse,
  ApiBlockSummary,
  ApiMetrics,
  ApiMetricsHistoryResponse,
  ApiMempoolSummary,
  ApiMempoolTxSummary,
  ApiMempoolTxListResponse,
  ApiNetworkResponse,
  ApiProposalDetail,
  ApiProposalListResponse,
  ApiProposalVote,
  ApiProposalVoteListResponse,
  ApiRawBlockResponse,
  ApiRawTxResponse,
  ApiRichListResponse,
  ApiSearchResponse,
  ApiStatus,
  ApiTxDetail,
  ApiTxListResponse,
  ApiUtxoListResponse,
  ApiValidatorDetail,
  ApiValidatorHistoryEntry,
  ApiValidatorListResponse,
  ApiValidatorSummary,
} from "./types";

export const API_BASE_PATH = "/api/v1";

function getApiBaseUrl(): string {
  if (typeof globalThis.location === "object" && typeof globalThis.location.origin === "string") {
    return new URL(API_BASE_PATH, globalThis.location.origin).toString();
  }

  return "http://127.0.0.1:3080" + API_BASE_PATH;
}

function getApiClient(): ApClient {
  return ApCreateClient({
    baseUrl: getApiBaseUrl(),
    fetch: globalThis.fetch,
  });
}

/** Carries HTTP status with API failures from the REST client. */
export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

function toApiError(error: ApApiError): ApiError {
  switch (error.kind) {
    case "availability":
    case "http":
      return new ApiError(error.status, error.message);
    case "contract":
    case "network":
      return new ApiError(500, error.message);
  }
}

async function unwrapResult<TValue>(promise: Promise<ApResult<TValue, ApApiError>>): Promise<TValue> {
  const result = await promise;
  const value = result.value();
  if (value !== undefined) {
    return value;
  }

  const error = result.error();
  if (error == null) {
    throw new ApiError(500, "Unknown error");
  }

  throw toApiError(error);
}

async function unwrapOptionalResult<TValue>(promise: Promise<ApResult<TValue, ApApiError>>): Promise<TValue | null> {
  const result = await promise;
  const value = result.value();
  if (value !== undefined) {
    return value;
  }

  const error = result.error();
  if (error?.kind === "http" && error.status === 404) {
    return null;
  }

  if (error == null) {
    throw new ApiError(500, "Unknown error");
  }

  throw toApiError(error);
}

function toNullableString(value: string | undefined): string | null {
  return value ?? null;
}

function toNullableNumber(value: number | undefined): number | null {
  return value ?? null;
}

function mapAddressSummary(summary: ApAddressSummary): ApiAddressSummary {
  return {
    address: summary.address,
    asOfHeight: summary.asOfHeight,
    balance: summary.balance,
    totalReceived: summary.totalReceived,
    totalSent: summary.totalSent,
    txCount: summary.txCount,
    firstSeen: toNullableString(summary.firstSeen),
    lastSeen: toNullableString(summary.lastSeen),
  };
}

function mapAddressTransactionSummary(summary: ApAddressTransactionSummary): ApiAddressTxListResponse["value"][number] {
  return {
    txHash: summary.txHash,
    blockTime: toNullableString(summary.blockTime),
    blockHeight: toNullableNumber(summary.blockHeight),
    blockPos: toNullableNumber(summary.blockPos),
    direction: summary.direction,
    txVersion: summary.txVersion,
    netValue: summary.netValue,
    txType: summary.txType,
    txSize: summary.txSize,
    valueOut: summary.valueOut,
    fee: toNullableNumber(summary.fee),
    tags: summary.tags,
  };
}

function mapAddressUtxo(utxo: ApAddressUtxo): ApiUtxoListResponse["value"][number] {
  return {
    txHash: utxo.txid,
    vout: utxo.vout,
    value: utxo.value,
    height: utxo.height,
    scriptPubKey: utxo.scriptPubKey,
  };
}

function mapBalancePage(page: ApBalancePage): ApiRichListResponse {
  return {
    asOfHeight: page.asOfHeight,
    includedSupply: page.includedSupply,
    supplyDefinition: page.supplyDefinition,
    totalCount: page.totalCount ?? null,
    value: page.value as readonly ApiRichListResponse["value"][number][],
    nextCursor: page.nextCursor ?? null,
  };
}

function mapTransactionSummary(summary: ApTransactionSummary): ApiTxListResponse["value"][number] {
  return {
    txHash: summary.txHash,
    blockHeight: summary.blockHeight,
    blockPos: summary.blockPos,
    txVersion: summary.txVersion,
    txType: summary.txType,
    lockTime: summary.lockTime,
    txSize: summary.txSize,
    valueOut: summary.valueOut,
    inputCount: summary.inputCount,
    outputCount: summary.outputCount,
    tags: summary.tags,
  };
}

function mapTransactionDetail(detail: ApTransactionDetail): ApiTxDetail {
  return {
    txHash: detail.txHash,
    blockHeight: toNullableNumber(detail.blockHeight),
    blockPos: toNullableNumber(detail.blockPos),
    txVersion: detail.txVersion,
    txType: detail.txType,
    lockTime: detail.lockTime,
    txSize: detail.txSize,
    valueOut: detail.valueOut,
    fee: toNullableNumber(detail.fee),
    inputCount: detail.inputCount,
    outputCount: detail.outputCount,
    tags: detail.tags,
    live: detail.live,
    payload: detail.payload,
    vin: detail.vin.map((input) => ({
      n: input.n,
      sequence: input.sequence,
      address: toNullableString(input.address),
      value: toNullableNumber(input.value),
      txid: toNullableString(input.txid),
      vout: toNullableNumber(input.vout),
      coinbase: toNullableString(input.coinbase),
      scriptSig: input.scriptSig,
    })),
    vout: detail.vout.map((output) => ({
      n: output.n,
      value: output.value,
      scriptPubKey: output.scriptPubKey,
      address: toNullableString(output.address),
      spent: output.spent,
    })),
  };
}

function mapValidatorSummary(summary: ApValidatorSummary): ApiValidatorSummary {
  return {
    proTxHash: summary.proTxHash,
    collateralHash: summary.collateralHash,
    collateralIndex: summary.collateralIndex,
    addresses: {
      collateral: toNullableString(summary.addresses.collateral),
      owner: toNullableString(summary.addresses.owner),
      payee: toNullableString(summary.addresses.payee),
      payout: toNullableString(summary.addresses.payout),
      voting: toNullableString(summary.addresses.voting),
    },
    live: {
      consecutivePayments: toNullableNumber(summary.live.consecutivePayments),
      lastPaidAt: toNullableString(summary.live.lastPaidAt),
      lastPaidHeight: toNullableNumber(summary.live.lastPaidHeight),
      pose:
        summary.live.pose == null
          ? null
          : {
              banHeight: toNullableNumber(summary.live.pose.banHeight),
              banScore: toNullableNumber(summary.live.pose.banScore),
              revivedHeight: toNullableNumber(summary.live.pose.revivedHeight),
            },
      revocationReason: toNullableNumber(summary.live.revocationReason),
      status: toNullableString(summary.live.status),
      type: toNullableString(summary.live.type),
      valid: summary.live.valid ?? null,
    },
    operatorPubKey: toNullableString(summary.operatorPubKey),
    operatorReward: summary.operatorReward,
    type: summary.type,
    status: summary.status,
    services: summary.services,
    registeredHeight: summary.registeredHeight,
    lastUpdatedHeight: summary.lastUpdatedHeight,
  };
}

function mapValidatorHistoryEntry(entry: ApValidatorHistoryEntry): ApiValidatorHistoryEntry {
  return {
    txHash: entry.txid,
    txType: entry.txType,
    payload: entry.payload,
    blockHeight: entry.blockHeight,
  };
}

function mapValidatorDetail(detail: ApValidatorDetail): ApiValidatorDetail {
  return {
    ...mapValidatorSummary(detail),
    asOfHeight: detail.asOfHeight,
    history: detail.history.map(mapValidatorHistoryEntry),
  };
}

function mapRawBlockResponse(response: ApRawBlockResponse): ApiRawBlockResponse {
  return {
    as_of_height: response.asOfHeight,
    rawblock: response.rawblock,
  };
}

function mapRawTxResponse(response: ApRawTxResponse): ApiRawTxResponse {
  return {
    as_of_height: response.asOfHeight ?? null,
    rawtx: response.rawtx,
  };
}

function mapNetworkResponse(snapshot: ApNetworkSnapshot): ApiNetworkResponse {
  return {
    networkInfo: snapshot.networkInfo as Readonly<Record<string, unknown>>,
    peerInfo: snapshot.peerInfo as readonly Readonly<Record<string, unknown>>[],
    peerCount: snapshot.peerCount,
    validators: snapshot.validators,
    updatedAt: snapshot.updatedAt,
  };
}

function mapProposalDetail(proposal: ApProposalDetail): ApiProposalDetail {
  return {
    hash: proposal.hash,
    collateralHash: toNullableString(proposal.collateralHash),
    name: proposal.name,
    url: toNullableString(proposal.url),
    paymentAddress: toNullableString(proposal.paymentAddress),
    paymentAmount: proposal.paymentAmount,
    startTime: toNullableString(proposal.startTime),
    endTime: toNullableString(proposal.endTime),
    votes: proposal.votes,
    fundingThreshold: proposal.fundingThreshold,
    meetsFundingThreshold: proposal.meetsFundingThreshold,
    funded: proposal.funded,
    valid: proposal.valid,
    creationTime: toNullableString(proposal.creationTime),
  };
}

function mapBlockSummary(summary: ApBlockSummary): ApiBlockSummary {
  return {
    blockHeight: summary.blockHeight,
    blockHash: summary.blockHash,
    prevBlockHash: summary.prevBlockHash,
    blockTime: summary.blockTime,
    bits: summary.bits,
    nonce: summary.nonce,
    blockVersion: summary.blockVersion,
    blockSize: summary.blockSize,
    txCount: summary.txCount,
    tags: summary.tags,
    live: summary.live,
  };
}

function mapBlockDetail(detail: ApBlockDetail): ApiBlockDetail {
  return {
    ...mapBlockSummary(detail),
    merkle: detail.merkle,
    chainwork: toNullableString(detail.chainwork),
    cbTx:
      detail.cbTx == null
        ? null
        : {
            height: detail.cbTx.height,
            merkleRootMnList: toNullableString(detail.cbTx.merkleRootMnList),
            merkleFlags: toNullableNumber(detail.cbTx.merkleFlags),
            bestClHeight: toNullableNumber(detail.cbTx.bestClHeight),
            bestClSignature: toNullableString(detail.cbTx.bestClSignature),
            creditPoolBalance: toNullableNumber(detail.cbTx.creditPoolBalance),
          },
    status: detail.status,
    orphanedAt: toNullableString(detail.orphanedAt),
  };
}

function mapMempoolTransactionSummary(summary: ApMempoolTransactionSummary): ApiMempoolTxSummary {
  return {
    txHash: summary.txHash,
    txSize: summary.txSize,
    firstSeenAt: summary.firstSeenAt,
    lastSeenAt: summary.lastSeenAt,
    txType: summary.txType,
    valueOut: summary.valueOut,
    tags: summary.tags,
  };
}

function mapProposalVote(vote: ApProposalVote): ApiProposalVote {
  return {
    proTxHash: vote.proTxHash,
    vote: vote.vote,
    voteTime: toNullableString(vote.voteTime),
  };
}

function mapSearchResponse(result: ApSearchResult): ApiSearchResponse {
  switch (result.type) {
    case "address":
      return result;
    case "block":
      return result;
    case "proposal":
      return result;
    case "tx":
      return result;
    case "validator":
      return result;
  }
}

function createCursorParams(
  limit?: number,
  cursor?: string
): { readonly limit?: number; readonly cursor?: string } | undefined {
  if (limit == null && cursor == null) {
    return undefined;
  }

  return {
    ...(limit == null ? {} : { limit }),
    ...(cursor == null ? {} : { cursor }),
  };
}

function createMetricsChartParams(
  blocks?: number,
  points?: number
): { readonly blocks?: number; readonly points?: number } | undefined {
  if (blocks == null && points == null) {
    return undefined;
  }

  return {
    ...(blocks == null ? {} : { blocks }),
    ...(points == null ? {} : { points }),
  };
}

function createValidatorListParams(
  limit?: number,
  cursor?: string,
  status?: string,
  validatorType?: string
): { readonly limit?: number; readonly cursor?: string; readonly status?: string; readonly type?: string } | undefined {
  if (limit == null && cursor == null && status == null && validatorType == null) {
    return undefined;
  }

  return {
    ...(limit == null ? {} : { limit }),
    ...(cursor == null ? {} : { cursor }),
    ...(status == null ? {} : { status }),
    ...(validatorType == null ? {} : { type: validatorType }),
  };
}

/** Lists blocks from newest to oldest. */
export async function getBlocks(limit?: number, cursor?: string): Promise<ApiBlockListResponse> {
  const page = await unwrapResult(getApiClient().blocks.list(createCursorParams(limit, cursor)));
  return {
    asOfHeight: page.asOfHeight,
    value: page.value.map(mapBlockSummary),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Loads one block by height or hash and returns null for HTTP 404. */
export async function getBlock(hashOrHeight: string | number): Promise<ApiBlockDetail | null> {
  const detail = await unwrapOptionalResult(getApiClient().blocks.get(String(hashOrHeight)));
  return detail == null ? null : mapBlockDetail(detail);
}

/** Lists transactions that belong to one block. */
export async function getBlockTxs(
  hashOrHeight: string | number,
  limit?: number,
  cursor?: string
): Promise<ApiTxListResponse> {
  const page = await unwrapResult(
    getApiClient().blocks.getTransactions(String(hashOrHeight), createCursorParams(limit, cursor))
  );
  return {
    asOfHeight: page.asOfHeight,
    value: page.value.map(mapTransactionSummary),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Loads the raw encoded block when the API exposes it. */
export async function getRawBlock(hashOrHeight: string | number): Promise<ApiRawBlockResponse> {
  const response = await unwrapResult(getApiClient().raw.getBlock(String(hashOrHeight)));
  return mapRawBlockResponse(response);
}

/** Lists transactions from newest to oldest. */
export async function getTransactions(limit?: number, cursor?: string): Promise<ApiTxListResponse> {
  const page = await unwrapResult(getApiClient().transactions.list(createCursorParams(limit, cursor)));
  return {
    asOfHeight: page.asOfHeight,
    value: page.value.map(mapTransactionSummary),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Loads one transaction by txid and returns null for HTTP 404. */
export async function getTransaction(txid: string): Promise<ApiTxDetail | null> {
  const detail = await unwrapOptionalResult(getApiClient().transactions.get(txid));
  return detail == null ? null : mapTransactionDetail(detail);
}

/** Loads the raw encoded transaction when the API exposes it. */
export async function getRawTransaction(txid: string): Promise<ApiRawTxResponse> {
  const response = await unwrapResult(getApiClient().raw.getTx(txid));
  return mapRawTxResponse(response);
}

/** Loads one address summary and returns null for HTTP 404. */
export async function getAddress(address: string): Promise<ApiAddressSummary | null> {
  const summary = await unwrapOptionalResult(getApiClient().addresses.get(address));
  return summary == null ? null : mapAddressSummary(summary);
}

/** Lists transactions that mention one address. */
export async function getAddressTxs(
  address: string,
  limit?: number,
  cursor?: string
): Promise<ApiAddressTxListResponse> {
  const page = await unwrapResult(getApiClient().addresses.getTransactions(address, createCursorParams(limit, cursor)));
  return {
    asOfHeight: page.asOfHeight,
    totalCount: page.totalCount ?? null,
    value: page.value.map(mapAddressTransactionSummary),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Lists UTXOs for one address when the index is enabled. */
export async function getAddressUtxos(address: string, limit?: number, cursor?: string): Promise<ApiUtxoListResponse> {
  const page = await unwrapResult(getApiClient().addresses.getUtxos(address, createCursorParams(limit, cursor)));
  return {
    asOfHeight: page.asOfHeight,
    value: page.value.map(mapAddressUtxo),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Lists richest addresses from the balances endpoint. */
export async function getRichList(limit?: number, cursor?: string): Promise<ApiRichListResponse> {
  const page = await unwrapResult(getApiClient().addresses.listBalances(createCursorParams(limit, cursor)));
  return mapBalancePage(page);
}

/** Loads the latest metrics snapshot for overview counters. */
export async function getMetricsLatest(): Promise<ApiMetrics> {
  const metrics = await unwrapResult(getApiClient().metrics.getLatest());
  return metrics as ApiMetrics;
}

/** Loads metrics history for dashboard charts and overlays. */
export async function getMetricsHistory(blocks?: number, points?: number): Promise<ApiMetricsHistoryResponse> {
  const chart = await unwrapResult(getApiClient().metrics.getChart(createMetricsChartParams(blocks, points)));
  return chart as ApiMetricsHistoryResponse;
}

/** Loads the current mempool summary snapshot. */
export async function getMempool(): Promise<ApiMempoolSummary> {
  const summary = await unwrapResult(getApiClient().live.getMempoolSummary());
  return summary as ApiMempoolSummary;
}

/** Lists recently seen mempool transactions. */
export async function getMempoolTxs(limit?: number, cursor?: string): Promise<ApiMempoolTxListResponse> {
  const page = await unwrapResult(getApiClient().live.listMempoolTransactions(createCursorParams(limit, cursor)));
  return {
    value: page.value.map(mapMempoolTransactionSummary),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Loads the current daemon network snapshot. */
export async function getNetwork(): Promise<ApiNetworkResponse> {
  const snapshot = await unwrapResult(getApiClient().live.getNetworkSnapshot());
  return mapNetworkResponse(snapshot);
}

/** Lists live governance proposals. */
export async function getProposals(limit?: number, cursor?: string): Promise<ApiProposalListResponse> {
  const page = await unwrapResult(getApiClient().governanceSearch.listProposals(createCursorParams(limit, cursor)));
  return {
    totalCount: page.totalCount ?? null,
    value: page.value.map(mapProposalDetail),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Loads one governance proposal and returns null for HTTP 404. */
export async function getProposal(hash: string): Promise<ApiProposalDetail | null> {
  const proposal = await unwrapOptionalResult(getApiClient().governanceSearch.getProposal(hash));
  return proposal == null ? null : mapProposalDetail(proposal);
}

/** Lists current votes for one governance proposal. */
export async function getProposalVotes(
  hash: string,
  limit?: number,
  cursor?: string
): Promise<ApiProposalVoteListResponse> {
  const page = await unwrapResult(
    getApiClient().governanceSearch.getProposalVotes(hash, createCursorParams(limit, cursor))
  );
  return {
    totalCount: page.totalCount ?? null,
    value: page.value.map(mapProposalVote),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Resolves one search query into the typed suggestion list returned by the API. */
export async function searchExplorer(query: string): Promise<readonly ApiSearchResponse[]> {
  const result = await unwrapOptionalResult(getApiClient().governanceSearch.search(query));
  return (result?.value ?? []).map(mapSearchResponse);
}

/** Lists validators with optional status and type filters. */
export async function getValidators(
  limit?: number,
  cursor?: string,
  status?: string,
  type?: string
): Promise<ApiValidatorListResponse> {
  const page = await unwrapResult(
    getApiClient().validators.list(createValidatorListParams(limit, cursor, status, type))
  );
  return {
    asOfHeight: page.asOfHeight,
    totalCount: page.totalCount ?? null,
    value: page.value.map(mapValidatorSummary),
    nextCursor: page.nextCursor ?? null,
  };
}

/** Loads one validator by ProTx hash and returns null for HTTP 404. */
export async function getValidator(proTxHash: string): Promise<ApiValidatorDetail | null> {
  const detail = await unwrapOptionalResult(getApiClient().validators.get(proTxHash));
  return detail == null ? null : mapValidatorDetail(detail);
}

/** Loads daemon and index status for shell health and route gating. */
export async function getStatus(): Promise<ApiStatus> {
  const status = await unwrapResult(getApiClient().status.getStatus());
  return status as ApiStatus;
}
