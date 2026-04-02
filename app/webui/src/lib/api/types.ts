export interface ApiErrorBody {
  readonly code: string;
  readonly message: string;
  readonly target?: string | null;
}

export interface ApiErrorResponse {
  readonly error: ApiErrorBody;
}

export interface ApiBlockTags {
  readonly orphan: boolean;
}

export interface ApiBlockLiveTags {
  readonly chainlock?: boolean | null;
}

export interface ApiBlockLive {
  readonly coreHeight?: number | null;
  readonly seenAt?: string | null;
  readonly tags?: ApiBlockLiveTags | null;
}

export interface ApiBlockSummary {
  readonly height: number;
  readonly hash: string;
  readonly previousblockhash: string;
  readonly time: string;
  readonly bits: string;
  readonly nonce: number;
  readonly version: number;
  readonly size: number;
  readonly txCount: number;
  readonly tags: ApiBlockTags;
  readonly live: ApiBlockLive;
}

export interface ApiBlockListResponse {
  readonly asOfHeight: number;
  readonly value: readonly ApiBlockSummary[];
  readonly nextCursor?: string | null;
}

export interface ApiCbTxFields {
  readonly height: number;
  readonly merkleRootMnList: string | null;
  readonly merkleFlags: number | null;
  readonly bestClHeight: number | null;
  readonly bestClSignature: string | null;
  readonly creditPoolBalance: number | null;
}

export interface ApiBlockDetail extends ApiBlockSummary {
  readonly merkleroot: string;
  readonly chainwork: string | null;
  readonly cbTx: ApiCbTxFields | null;
  readonly status: string;
  readonly orphanedAt?: string | null;
}

export interface ApiRawBlockResponse {
  readonly as_of_height: number;
  readonly rawblock: string;
}

export interface ApiTxTags {
  readonly mined: boolean;
  readonly coinbase: boolean;
}

export interface ApiTxLiveTags {
  readonly islock?: boolean | null;
  readonly mempool?: boolean | null;
}

export interface ApiTxLive {
  readonly tags?: ApiTxLiveTags | null;
}

export interface ApiTxSummary {
  readonly txid: string;
  readonly blockHeight: number;
  readonly blockPos: number;
  readonly version: number;
  readonly txType: string;
  readonly lockTime: number;
  readonly size: number;
  readonly valueOut: number;
  readonly inputCount: number;
  readonly outputCount: number;
  readonly tags: ApiTxTags;
}

export interface ApiTxListResponse {
  readonly asOfHeight: number;
  readonly value: readonly ApiTxSummary[];
  readonly nextCursor?: string | null;
}

export interface ApiTxScriptSig {
  readonly hex?: string;
  readonly asm?: string;
}

export interface ApiTxScriptPubKey {
  readonly hex: string;
  readonly asm?: string;
  readonly type?: string | null;
}

export interface ApiTxInput {
  readonly n: number;
  readonly sequence: number;
  readonly address: string | null;
  readonly value: number | null;
  readonly txid: string | null;
  readonly vout: number | null;
  readonly coinbase: string | null;
  readonly scriptSig: ApiTxScriptSig;
}

export interface ApiTxOutput {
  readonly n: number;
  readonly value: number;
  readonly scriptPubKey: ApiTxScriptPubKey;
  readonly address: string | null;
  readonly spent: boolean;
}

export interface ApiTxDetail {
  readonly txid: string;
  readonly blockHeight: number | null;
  readonly blockPos: number | null;
  readonly version: number;
  readonly txType: string;
  readonly lockTime: number;
  readonly size: number;
  readonly valueOut: number;
  readonly fee: number | null;
  readonly inputCount: number;
  readonly outputCount: number;
  readonly tags: ApiTxTags;
  readonly live: ApiTxLive;
  readonly payload?: unknown;
  readonly vin: readonly ApiTxInput[];
  readonly vout: readonly ApiTxOutput[];
}

export interface ApiRawTxResponse {
  readonly as_of_height: number | null;
  readonly rawtx: string;
}

export interface ApiBroadcastRequest {
  readonly rawtx: string;
}

export interface ApiBroadcastResponse {
  readonly txid: string;
}

export interface ApiAddressSummary {
  readonly address: string;
  readonly asOfHeight: number;
  readonly balance: number;
  readonly totalReceived: number;
  readonly totalSent: number;
  readonly txCount: number;
  readonly firstSeen: string | null;
  readonly lastSeen: string | null;
}

export interface ApiAddressTxSummary {
  readonly txid: string;
  readonly blockTime: string | null;
  readonly blockHeight: number | null;
  readonly blockPos: number | null;
  readonly direction: "in" | "out";
  readonly version: number;
  readonly netValue: number;
  readonly txType: string;
  readonly size: number;
  readonly valueOut: number;
  readonly fee: number | null;
  readonly tags: ApiTxTags;
}

export interface ApiAddressTxListResponse {
  readonly asOfHeight: number;
  readonly totalCount: number | null;
  readonly value: readonly ApiAddressTxSummary[];
  readonly nextCursor?: string | null;
}

export interface ApiUtxo {
  readonly txid: string;
  readonly vout: number;
  readonly value: number;
  readonly height: number;
  readonly scriptPubKey: string;
}

export interface ApiUtxoListResponse {
  readonly asOfHeight: number;
  readonly value: readonly ApiUtxo[];
  readonly nextCursor?: string | null;
}

export interface ApiRichListEntry {
  readonly rank: number;
  readonly address: string;
  readonly balance: number;
  readonly pctSupply: number;
  readonly totalReceived: number;
  readonly totalSent: number;
  readonly txCount: number;
}

export interface ApiRichListResponse {
  readonly asOfHeight: number;
  readonly includedSupply: number;
  readonly supplyDefinition: string;
  readonly totalCount: number | null;
  readonly value: readonly ApiRichListEntry[];
  readonly nextCursor?: string | null;
}

export interface ApiMetrics {
  readonly height: number;
  readonly txCount: number;
  readonly utxosSpendable: number;
  readonly utxosUnspendable: number;
  readonly blockchainSize: number;
  readonly creditPoolBalance: number | null;
  readonly circulatingSupply: number;
  readonly difficulty: number;
  readonly hashrate: number;
}

export interface ApiMetricsHistoryResponse {
  readonly asOfHeight: number;
  readonly startHeight: number;
  readonly endHeight: number;
  readonly bucketBlocks: number;
  readonly value: readonly ApiMetrics[];
}

export interface ApiMempoolSummary {
  readonly count: number;
  readonly totalBytes: number;
  readonly usageBytes: number | null;
  readonly maxBytes: number | null;
  readonly updatedAt: string;
  readonly recentHashes: readonly string[];
}

export interface ApiMempoolTxSummary {
  readonly txid: string;
  readonly size: number;
  readonly firstSeenAt: string;
  readonly lastSeenAt: string;
  readonly txType?: string | null;
  readonly valueOut?: number | null;
  readonly tags: ApiTxTags;
}

export interface ApiMempoolTxListResponse {
  readonly value: readonly ApiMempoolTxSummary[];
  readonly nextCursor?: string | null;
}

export type ApiNetworkPeerInfo = Readonly<Record<string, unknown>>;

export interface ApiNetworkResponse {
  readonly networkInfo: Readonly<Record<string, unknown>>;
  readonly peerInfo: readonly ApiNetworkPeerInfo[];
  readonly peerCount: number;
  readonly updatedAt: string;
}

export interface ApiSearchResponse {
  readonly type: string;
  readonly id: string;
  readonly status?: string | null;
  readonly blockTags?: ApiBlockTags | null;
  readonly txTags?: ApiTxTags | null;
}

export interface ApiProposalDetail {
  readonly hash: string;
  readonly collateralHash: string | null;
  readonly name: string;
  readonly url: string | null;
  readonly paymentAddress: string | null;
  readonly paymentAmount: number;
  readonly startTime: string | null;
  readonly endTime: string | null;
  readonly yesCount: number;
  readonly noCount: number;
  readonly abstainCount: number;
  readonly voteCount: number;
  readonly totalNodes: number;
  readonly enabledNodes: number;
  readonly participatingNodes: number;
  readonly fundingThreshold: number;
  readonly funded: boolean;
  readonly valid: boolean;
  readonly creationTime: string | null;
}

export interface ApiProposalListResponse {
  readonly totalCount: number | null;
  readonly value: readonly ApiProposalDetail[];
  readonly nextCursor?: string | null;
}

export interface ApiProposalVote {
  readonly proTxHash: string;
  readonly vote: string;
  readonly voteTime: string | null;
}

export interface ApiProposalVoteListResponse {
  readonly totalCount: number | null;
  readonly value: readonly ApiProposalVote[];
  readonly nextCursor?: string | null;
}

export interface ApiValidatorSummary {
  readonly proTxHash: string;
  readonly collateralHash: string;
  readonly collateralIndex: number;
  readonly addresses: ApiValidatorAddresses;
  readonly live: ApiValidatorLive;
  readonly operatorPubKey: string | null;
  readonly operatorReward: number;
  readonly type: string;
  readonly status: string;
  readonly services: ApiValidatorServices;
  readonly registeredHeight: number;
  readonly lastUpdatedHeight: number;
}

export interface ApiValidatorListResponse {
  readonly asOfHeight: number;
  readonly totalCount: number | null;
  readonly value: readonly ApiValidatorSummary[];
  readonly nextCursor?: string | null;
}

export interface ApiValidatorHistoryEntry {
  readonly txid: string;
  readonly txType: string;
  readonly payload: unknown;
  readonly blockHeight: number;
}

export interface ApiValidatorAddresses {
  readonly collateral?: string | null;
  readonly owner?: string | null;
  readonly payout?: string | null;
  readonly voting?: string | null;
}

export interface ApiValidatorServices {
  readonly coreP2p: readonly string[];
  readonly platformP2p: readonly string[];
  readonly platformHttps: readonly string[];
}

export interface ApiValidatorLive {
  readonly consecutivePayments?: number | null;
  readonly currentStatus?: string | null;
  readonly enabled?: boolean | null;
  readonly lastPaidAt?: string | null;
  readonly lastPaidHeight?: number | null;
  readonly poseBanHeight?: number | null;
  readonly posePenalty?: number | null;
  readonly poseRevivedHeight?: number | null;
  readonly revocationReason?: number | null;
}

export interface ApiValidatorDetail extends ApiValidatorSummary {
  readonly asOfHeight: number;
  readonly history: readonly ApiValidatorHistoryEntry[];
}

export interface ApiIndexStatus {
  readonly indexedHeight: number;
  readonly updatedAt: string;
}

export interface ApiStatus {
  readonly syncStatus: string;
  readonly blockHeight: number;
  readonly indices: Readonly<Record<string, ApiIndexStatus>>;
  readonly version: string;
}
