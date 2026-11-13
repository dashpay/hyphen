//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! OpenAPI schema types and the top-level `ApiDoc` document definition.
//!
//! All monetary values are denominated in the network's smallest base unit.
//! All hashes and byte arrays are hex-encoded strings.

use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};

use crate::index::block::rest as block_rest;
use crate::index::metrics::rest as metrics_rest;
use crate::index::spent::rest as spent_rest;
use crate::index::tx::rest as tx_rest;
use crate::index::validator::rest as validator_rest;
use crate::live::gov::rest as gov_rest;
use crate::live::mempool::rest as mempool_rest;
use crate::live::network::rest as network_rest;
use crate::live::stream::rest as stream_rest;
use crate::rest::search;
use crate::rest::status;

/// Standard JSON error response body.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
  pub code: String,
  pub message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub target: Option<String>,
}

/// Standard JSON error response envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
  pub error: ErrorBody,
}

/// Summary of a single block (list view).
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlockTags {
  pub orphan: bool,
}

/// Nullable live tag summary for block overlay routes.
#[derive(Debug, Default, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlockLiveTags {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub chainlock: Option<bool>,
}

/// Nullable live overlay for block routes.
#[derive(Debug, Default, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlockLive {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub tags: Option<BlockLiveTags>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub core_height: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub seen_at: Option<String>,
}

/// Summary of a single block (list view).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockSummary {
  #[serde(rename = "blockHeight")]
  pub height: i32,
  #[serde(rename = "blockHash")]
  pub hash: String,
  #[serde(rename = "prevBlockHash")]
  pub previousblockhash: String,
  #[serde(rename = "blockTime")]
  pub time: String,
  pub bits: String,
  pub nonce: i64,
  #[serde(rename = "blockVersion")]
  pub version: i32,
  #[serde(rename = "blockSize")]
  pub size: i32,
  pub tx_count: i32,
  pub tags: BlockTags,
  pub live: BlockLive,
}

/// Indexed page envelope for block summaries.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockListResponse {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<BlockSummary>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Coinbase special transaction fields.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CbTxFields {
  pub height: i32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub merkle_root_mn_list: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub merkle_flags: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub best_cl_height: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub best_cl_signature: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub credit_pool_balance: Option<i64>,
}

/// Full block detail.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockDetail {
  pub as_of_height: i32,
  #[serde(rename = "blockHeight")]
  pub height: i32,
  #[serde(rename = "blockHash")]
  pub hash: String,
  #[serde(rename = "prevBlockHash")]
  pub previousblockhash: String,
  #[serde(rename = "merkle")]
  pub merkleroot: String,
  #[serde(rename = "blockTime")]
  pub time: String,
  pub bits: String,
  pub nonce: i64,
  #[serde(rename = "blockVersion")]
  pub version: i32,
  #[serde(rename = "blockSize")]
  pub size: i32,
  pub tx_count: i32,
  pub status: String,
  pub tags: BlockTags,
  pub live: BlockLive,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub orphaned_at: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub chainwork: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub cb_tx: Option<CbTxFields>,
}

/// Raw block hex.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RawBlockResponse {
  pub as_of_height: i32,
  pub source: RawValueSource,
  pub rawblock: String,
}

/// Transaction summary (list view).
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TxTags {
  pub mined: bool,
  pub coinbase: bool,
}

/// Nullable live tag summary for transaction overlay routes.
#[derive(Debug, Default, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TxLiveTags {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub mempool: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub islock: Option<bool>,
}

/// Nullable live overlay for transaction routes.
#[derive(Debug, Default, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TxLive {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub tags: Option<TxLiveTags>,
}

/// Transaction summary (list view).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxSummary {
  #[serde(rename = "txHash")]
  pub txid: String,
  pub block_height: i32,
  pub block_pos: i32,
  #[serde(rename = "txVersion")]
  pub version: i16,
  pub tx_type: String,
  /// Wide enough for the full u32 nLockTime range.
  pub lock_time: i64,
  #[serde(rename = "txSize")]
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
  pub tags: TxTags,
}

/// Indexed page envelope for transaction summaries.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxListResponse {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<TxSummary>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Indexed page envelope for one block's transactions.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockTxListResponse {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<TxSummary>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// A transaction input.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxInput {
  pub n: i32,
  pub script_sig: TxScriptSig,
  pub sequence: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub address: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub value: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[serde(rename = "txHash")]
  pub txid: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub vout: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub coinbase: Option<String>,
}

/// Transaction input script signature.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxScriptSig {
  pub hex: String,
}

/// A transaction output.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxOutput {
  pub n: i32,
  pub value: i64,
  pub script_pub_key: TxScriptPubKey,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub address: Option<String>,
  pub spent: bool,
}

/// Transaction output script public key.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxScriptPubKey {
  pub hex: String,
}

/// Full transaction detail.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TxDetail {
  pub as_of_height: Option<i32>,
  #[serde(rename = "txHash")]
  pub txid: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub block_height: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub block_pos: Option<i32>,
  #[serde(rename = "txVersion")]
  pub version: i16,
  pub tx_type: String,
  /// Wide enough for the full u32 nLockTime range.
  pub lock_time: i64,
  #[serde(rename = "txSize")]
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub fee: Option<i64>,
  pub tags: TxTags,
  pub live: TxLive,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub payload: Option<serde_json::Value>,
  pub vin: Vec<TxInput>,
  pub vout: Vec<TxOutput>,
}

/// Raw transaction hex.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RawTxResponse {
  pub as_of_height: Option<i32>,
  pub source: RawValueSource,
  pub rawtx: String,
}

/// Source class for one raw payload response.
#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RawValueSource {
  Indexed,
  Live,
}

/// Address balance summary.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddressSummary {
  pub as_of_height: i32,
  pub address: String,
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_seen: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_seen: Option<String>,
}

/// Address transaction list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AddressTxDirection {
  In,
  Out,
}

/// Address transaction list item.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddressTxSummary {
  #[serde(rename = "txHash")]
  pub txid: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub block_height: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub block_pos: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub block_time: Option<String>,
  #[serde(rename = "txVersion")]
  pub version: i16,
  pub tx_type: String,
  #[serde(rename = "txSize")]
  pub size: i32,
  pub value_out: i64,
  pub net_value: i64,
  pub direction: AddressTxDirection,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub fee: Option<i64>,
  pub tags: TxTags,
}

/// Indexed page envelope for address transaction history.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddressTxListResponse {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<AddressTxSummary>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// An unspent transaction output.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Utxo {
  #[serde(rename = "txHash")]
  pub txid: String,
  pub vout: i32,
  pub value: i64,
  pub height: i32,
  pub script_pub_key: String,
}

/// Indexed page envelope for address UTXOs.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UtxoListResponse {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<Utxo>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Address balances entry.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BalanceEntry {
  pub rank: i64,
  pub address: String,
  pub balance: i64,
  pub pct_supply: f64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
}

/// Additional context for one ranked address balance page.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BalancePageMeta {
  pub included_supply: i64,
  pub supply_definition: String,
}

/// Indexed page envelope for ranked address balances.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BalanceResponse {
  /// Canonical height this page was read at.
  ///
  /// Per page, not per listing. The rich list is maintained in place, so paging
  /// across several requests is not a consistent snapshot — an address that
  /// crosses a page boundary between requests can appear twice or not at all.
  pub as_of_height: i32,
  /// How many entries this listing holds, and can therefore page through.
  ///
  /// The rich list is served from a side table holding only the addresses above
  /// the configured floor, so this is that table's size. It used to be the
  /// chain-wide positive-balance count, which is far larger and left most of
  /// the advertised total unreachable: the cursor went terminal at the floor.
  pub total_count: i64,
  /// Addresses holding any positive balance, chain-wide.
  ///
  /// Reported separately because it is not the size of this listing. It is the
  /// number `total_count` used to carry.
  pub positive_balance_count: i64,
  pub value: Vec<BalanceEntry>,
  pub meta: BalancePageMeta,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Block metrics snapshot.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockMetrics {
  pub height: i32,
  pub tx_count: i64,
  pub utxos_spendable: i64,
  pub utxos_unspendable: i64,
  pub blockchain_size: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub credit_pool_balance: Option<i64>,
  pub circulating_supply: i64,
  pub difficulty: f64,
  pub hashrate: f64,
}

/// Block-ranged metrics chart response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricsChartResponse {
  pub as_of_height: i32,
  pub start_height: i32,
  pub end_height: i32,
  pub bucket_blocks: i32,
  pub value: Vec<BlockMetrics>,
}

/// Live mempool summary snapshot.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MempoolResponse {
  pub count: i64,
  pub total_bytes: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub usage_bytes: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub max_bytes: Option<i64>,
  pub updated_at: String,
  pub recent_hashes: Vec<String>,
}

/// Recently seen mempool transaction summary.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MempoolTxSummary {
  #[serde(rename = "txHash")]
  pub txid: String,
  pub tx_type: String,
  #[serde(rename = "txSize")]
  pub size: usize,
  pub value_out: i64,
  pub first_seen_at: String,
  pub last_seen_at: String,
  pub tags: TxTags,
}

/// Live page envelope for recent mempool transactions.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MempoolTxListResponse {
  pub total_count: i64,
  pub value: Vec<MempoolTxSummary>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Current network snapshot.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfoResponse {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub version: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub subversion: Option<String>,
  #[serde(alias = "protocolversion")]
  #[serde(skip_serializing_if = "Option::is_none")]
  pub protocol_version: Option<i64>,
  #[serde(alias = "timeoffset")]
  #[serde(skip_serializing_if = "Option::is_none")]
  pub time_offset: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub connections: Option<i64>,
  #[serde(alias = "relayfee")]
  #[serde(skip_serializing_if = "Option::is_none")]
  pub relay_fee: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub warnings: Option<String>,
}

/// Typed peer summary inside the live network snapshot.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfoResponse {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub id: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub addr: Option<String>,
  #[serde(alias = "addrlocal")]
  #[serde(skip_serializing_if = "Option::is_none")]
  pub addr_local: Option<String>,
  #[serde(alias = "subver")]
  #[serde(skip_serializing_if = "Option::is_none")]
  pub subversion: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub inbound: Option<bool>,
  #[serde(alias = "startingheight")]
  #[serde(skip_serializing_if = "Option::is_none")]
  pub starting_height: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub synced_headers: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub synced_blocks: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub connection_type: Option<String>,
}

/// Current network snapshot.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkValidatorClassSummary {
  pub total: i64,
  pub status: std::collections::HashMap<String, i64>,
}

/// Validator aggregates attached to the live network snapshot.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkValidatorSummary {
  pub regular: NetworkValidatorClassSummary,
  pub evonode: NetworkValidatorClassSummary,
}

/// Current network snapshot.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkResponse {
  pub network_info: NetworkInfoResponse,
  pub peer_info: Vec<PeerInfoResponse>,
  pub peer_count: i64,
  pub validators: NetworkValidatorSummary,
  pub updated_at: String,
}

/// Governance proposal summary or detail.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProposalVoteSummary {
  pub yes: i64,
  pub no: i64,
  pub abstain: i64,
}

/// Governance proposal summary or detail.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProposalDetail {
  pub hash: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub collateral_hash: Option<String>,
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub payment_address: Option<String>,
  pub payment_amount: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub start_time: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub end_time: Option<String>,
  pub votes: ProposalVoteSummary,
  pub funding_threshold: i64,
  pub meets_funding_threshold: bool,
  pub funded: bool,
  pub valid: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub creation_time: Option<String>,
}

/// Live page envelope for governance proposals.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProposalListResponse {
  pub total_count: i64,
  pub value: Vec<ProposalDetail>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// One governance vote summary.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProposalVote {
  pub pro_tx_hash: String,
  pub vote: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub vote_time: Option<String>,
}

/// Live page envelope for governance proposal votes.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProposalVoteListResponse {
  pub total_count: i64,
  pub value: Vec<ProposalVote>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Search result variant for a block identifier.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchBlockResult {
  /// Canonical display-order block hash for the matched block.
  pub block_hash: String,
  pub status: String,
  pub block_tags: BlockTags,
}

/// Search result variant for a transaction identifier.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchTxResult {
  /// Canonical display-order transaction id for the matched transaction.
  pub tx_hash: String,
  pub tx_tags: TxTags,
}

/// Search result variant for an address.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchAddressResult {
  pub address: String,
}

/// Search result variant for a validator identifier.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchValidatorResult {
  /// Canonical display-order ProTx hash for the matched validator.
  pub pro_tx_hash: String,
  pub status: String,
}

/// Search result variant for a governance proposal identifier.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchProposalResult {
  pub hash: String,
}

/// One typed global search result.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SearchResponse {
  Block(SearchBlockResult),
  Tx(SearchTxResult),
  Address(SearchAddressResult),
  Validator(SearchValidatorResult),
  Proposal(SearchProposalResult),
}

/// Capped autocomplete-style global search result list.
#[derive(Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchListResponse {
  pub value: Vec<SearchResponse>,
}

/// Validator summary (list item).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorAddresses {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub collateral: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub owner: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub voting: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub payout: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub payee: Option<String>,
}

/// Validator service endpoints grouped by purpose.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorServices {
  pub core_p2p: Vec<String>,
  pub platform_p2p: Vec<String>,
  pub platform_https: Vec<String>,
}

/// Nullable live overlay for validator routes.
#[derive(Debug, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorPose {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub ban_score: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub revived_height: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub ban_height: Option<i32>,
}

/// Nullable live overlay for validator routes.
#[derive(Debug, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
// nosemgrep: contrib.r2c.daemon-rest-openapi-validator-live-must-not-duplicate-status -- existing API field, served to the web UI
pub struct ValidatorLive {
  #[serde(rename = "type")]
  #[serde(skip_serializing_if = "Option::is_none")]
  // nosemgrep: contrib.r2c.daemon-validator-live-must-not-duplicate-indexed-fields -- live RPC value, served to the web UI
  pub validator_type: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub valid: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  // nosemgrep: contrib.r2c.daemon-validator-live-must-not-duplicate-indexed-fields -- live RPC value, served to the web UI
  pub status: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_paid_height: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_paid_at: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub consecutive_payments: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub pose: Option<ValidatorPose>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub revocation_reason: Option<i32>,
}

/// Validator summary (list item).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorSummary {
  pub pro_tx_hash: String,
  pub collateral_hash: String,
  pub collateral_index: i32,
  pub addresses: ValidatorAddresses,
  pub live: ValidatorLive,
  pub operator_pub_key: Option<String>,
  pub operator_reward: i32,
  #[serde(rename = "type")]
  pub validator_type: String,
  pub status: String,
  pub services: ValidatorServices,
  pub registered_height: i32,
  pub last_updated_height: i32,
}

/// Indexed page envelope for validator summaries.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorListResponse {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<ValidatorSummary>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Full validator detail with history.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorHistoryEntry {
  #[serde(rename = "txHash")]
  pub txid: String,
  pub tx_type: String,
  pub payload: serde_json::Value,
  pub block_height: i32,
}

/// Full validator detail with history.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorDetail {
  pub as_of_height: i32,
  pub pro_tx_hash: String,
  pub collateral_hash: String,
  pub collateral_index: i32,
  pub addresses: ValidatorAddresses,
  pub live: ValidatorLive,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub operator_pub_key: Option<String>,
  pub operator_reward: i32,
  #[serde(rename = "type")]
  pub validator_type: String,
  pub status: String,
  pub services: ValidatorServices,
  pub registered_height: i32,
  pub last_updated_height: i32,
  pub history: Vec<ValidatorHistoryEntry>,
}

/// Per-stage indexing progress.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StageStatus {
  pub indexed_height: i32,
  pub updated_at: String,
}

/// Daemon sync status.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
  pub sync_status: String,
  /// Index-maintenance regime: `realtime` maintains every index per block,
  /// `stepped` defers the open partition's query-only indexes until it closes.
  /// Results are identical either way; only the cost of producing them moves.
  pub sync_mode: String,
  pub block_height: i32,
  pub indices: std::collections::HashMap<String, StageStatus>,
  pub version: String,
}

/// Generated OpenAPI document for the public REST API.
#[derive(Debug, OpenApi)]
#[openapi(
  info(
    title = "Hyphen API",
    version = "1",
    description = "Knowledge explorer REST API. Monetary values use the network's smallest base unit. Hashes are hex-encoded strings. Endpoint descriptions indicate whether each route is backed by a committed index snapshot, a live capability snapshot, or a derived server-side view. Optional index and live routes return 501 when the required capability is disabled and 503 until the required dataset is ready.",
    license(name = "MIT"),
  ),
  paths(
    block_rest::list_blocks,
    block_rest::get_block,
    block_rest::get_block_txs,
    block_rest::get_rawblock,
    tx_rest::list_txs,
    tx_rest::get_tx,
    tx_rest::get_rawtx,
    spent_rest::get_address,
    spent_rest::get_address_txs,
    spent_rest::get_address_utxos,
    spent_rest::get_balances,
    metrics_rest::latest,
    metrics_rest::chart,
    mempool_rest::summary,
    mempool_rest::recent_txs,
    network_rest::snapshot,
    gov_rest::list_proposals,
    gov_rest::get_proposal,
    gov_rest::get_votes,
    search::search,
    stream_rest::events,
    validator_rest::list_validators,
    validator_rest::get_validator,
    status::status,
  ),
  components(schemas(
    ErrorResponse,
    BlockSummary,
    BlockTags,
    BlockLiveTags,
    BlockLive,
    BlockListResponse,
    CbTxFields,
    BlockDetail,
    RawBlockResponse,
    RawValueSource,
    TxSummary,
    TxTags,
    TxLiveTags,
    TxLive,
    TxListResponse,
    BlockTxListResponse,
    TxInput,
    TxOutput,
    TxDetail,
    RawTxResponse,
    AddressSummary,
    AddressTxSummary,
    AddressTxListResponse,
    Utxo,
    UtxoListResponse,
    BalanceEntry,
    BalanceResponse,
    BlockMetrics,
    MetricsChartResponse,
    MempoolResponse,
    MempoolTxSummary,
    MempoolTxListResponse,
    NetworkInfoResponse,
    PeerInfoResponse,
    NetworkValidatorClassSummary,
    NetworkValidatorSummary,
    NetworkResponse,
    ProposalVoteSummary,
    ProposalDetail,
    ProposalListResponse,
    ProposalVote,
    ProposalVoteListResponse,
    SearchListResponse,
    SearchResponse,
    crate::live::stream::event::MempoolTxEvent,
    crate::live::stream::event::ChainLockEvent,
    crate::live::stream::event::GovernanceObjectEvent,
    crate::live::stream::event::GovernanceVoteEvent,
    crate::live::stream::event::NetworkEvent,
    crate::live::stream::event::LiveEvent,
    ValidatorSummary,
    ValidatorListResponse,
    ValidatorDetail,
    ValidatorLive,
    ValidatorPose,
    StageStatus,
    StatusResponse,
  )),
  tags(
    (name = "Blocks", description = "Canonical block views backed by committed block-index snapshots. These are not live RPC passthrough views."),
    (name = "Transactions", description = "Canonical transaction views backed by the transaction index, with explicit live mempool and ISLock fallbacks only where documented."),
    (name = "Addresses", description = "Address history, UTXO, and ranked balance views backed by the optional address/spent index. Some fields are derived aggregations rather than raw Core RPC outputs."),
    (name = "Metrics", description = "Derived chain statistics backed by the optional metrics index."),
    (name = "Mempool", description = "Live mempool views backed by the live mempool capability and not by indexed chain history."),
    (name = "Live", description = "Streaming HTTP feed of live event payloads sourced from enabled live pipelines."),
    (name = "Network", description = "Live network snapshot backed by the live network capability."),
    (name = "Governance", description = "Live governance proposal and vote views backed by the live governance capability, not by a historical governance index."),
    (name = "Search", description = "Cross-domain resolver that combines committed indexed datasets with enabled live capabilities."),
    (name = "Validators", description = "Validator registry views backed by the optional validator index."),
    (name = "Status", description = "Server-specific readiness and index capability status."),
  )
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
  use utoipa::OpenApi;

  use super::ApiDoc;

  #[test]
  fn openapi_includes_live_endpoints_and_tag() {
    let doc = ApiDoc::openapi();
    let paths = &doc.paths.paths;
    assert!(paths.contains_key("/api/v1/metrics/chart"));
    assert!(!paths.contains_key("/api/v1/metrics/history"));
    assert!(paths.contains_key("/api/v1/mempool"));
    assert!(paths.contains_key("/api/v1/mempool/txs"));
    assert!(paths.contains_key("/api/v1/network"));
    assert!(paths.contains_key("/api/v1/proposals"));
    assert!(paths.contains_key("/api/v1/search"));
    assert!(paths.contains_key("/api/v1/stream"));

    let tags = doc.tags.expect("openapi tags should exist");
    assert!(tags.iter().any(|tag| tag.name == "Live"));
    assert!(tags.iter().any(|tag| tag.name == "Mempool"));
    assert!(tags.iter().any(|tag| tag.name == "Governance"));
  }

  #[test]
  fn openapi_registers_live_stream_and_tag_schemas() {
    let doc = ApiDoc::openapi();
    let schemas = &doc
      .components
      .expect("openapi components should exist")
      .schemas;

    for name in [
      "ErrorBody",
      "LiveEvent",
      "MempoolTxEvent",
      "ChainLockEvent",
      "GovernanceObjectEvent",
      "GovernanceVoteEvent",
      "NetworkEvent",
      "BlockTags",
      "TxTags",
      "MempoolTxSummary",
      "MetricsChartResponse",
      "SearchResponse",
    ] {
      assert!(
        schemas.contains_key(name),
        "expected OpenAPI schema {name} to be registered"
      );
    }
  }

  #[test]
  fn openapi_raw_response_schemas_use_camel_case_without_live_overlay() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");

    for name in ["RawBlockResponse", "RawTxResponse"] {
      let properties = doc["components"]["schemas"][name]["properties"]
        .as_object()
        .expect("raw schema properties should exist");
      assert!(properties.contains_key("asOfHeight"));
      assert!(properties.contains_key("source"));
      assert!(!properties.contains_key("as_of_height"));
      assert!(!properties.contains_key("live"));
    }
  }

  #[test]
  fn openapi_validator_live_schema_omits_duplicate_status_field() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");
    let properties = doc["components"]["schemas"]["ValidatorLive"]["properties"]
      .as_object()
      .expect("validator live properties should exist");
    assert!(properties.contains_key("type"));
    assert!(properties.contains_key("valid"));
    assert!(properties.contains_key("status"));
    assert!(properties.contains_key("lastPaidHeight"));
    assert!(properties.contains_key("lastPaidAt"));
    assert!(properties.contains_key("pose"));
  }

  #[test]
  fn openapi_address_tx_schema_exposes_address_relative_fields() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");
    let properties = doc["components"]["schemas"]["AddressTxSummary"]["properties"]
      .as_object()
      .expect("address tx properties should exist");
    assert!(properties.contains_key("blockTime"));
    assert!(properties.contains_key("netValue"));
    assert!(properties.contains_key("direction"));
    let direction = &doc["components"]["schemas"]["AddressTxDirection"]["enum"];
    let values = direction.as_array().expect("direction enum should exist");
    assert_eq!(values.len(), 2);
    assert!(values.iter().any(|value| value == "in"));
    assert!(values.iter().any(|value| value == "out"));
  }

  #[test]
  fn openapi_network_response_uses_typed_nested_schemas() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");
    let properties = doc["components"]["schemas"]["NetworkResponse"]["properties"]
      .as_object()
      .expect("network response properties should exist");
    assert!(properties["networkInfo"]["$ref"]
      .as_str()
      .is_some_and(|value| value.ends_with("/NetworkInfoResponse")));
    assert!(properties["peerInfo"]["items"]["$ref"]
      .as_str()
      .is_some_and(|value| value.ends_with("/PeerInfoResponse")));
  }

  #[test]
  fn openapi_indexed_page_schemas_share_one_envelope_shape() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");

    for name in [
      "BlockListResponse",
      "TxListResponse",
      "BlockTxListResponse",
      "AddressTxListResponse",
      "UtxoListResponse",
      "BalanceResponse",
      "ValidatorListResponse",
    ] {
      let properties = doc["components"]["schemas"][name]["properties"]
        .as_object()
        .expect("indexed page properties should exist");
      assert!(
        properties.contains_key("asOfHeight"),
        "indexed page {name} must include asOfHeight"
      );
      assert!(
        properties.contains_key("totalCount"),
        "indexed page {name} must include totalCount"
      );
      assert!(
        properties.contains_key("value"),
        "indexed page {name} must include value"
      );
      assert!(
        properties.contains_key("nextCursor"),
        "indexed page {name} must include nextCursor"
      );
    }
  }

  #[test]
  fn openapi_live_page_schemas_omit_index_snapshot_height() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");

    for name in [
      "MempoolTxListResponse",
      "ProposalListResponse",
      "ProposalVoteListResponse",
    ] {
      let properties = doc["components"]["schemas"][name]["properties"]
        .as_object()
        .expect("live page properties should exist");
      assert!(
        !properties.contains_key("asOfHeight"),
        "live page {name} must not include asOfHeight"
      );
      assert!(
        properties.contains_key("totalCount"),
        "live page {name} must include totalCount"
      );
      assert!(
        properties.contains_key("value"),
        "live page {name} must include value"
      );
      assert!(
        properties.contains_key("nextCursor"),
        "live page {name} must include nextCursor"
      );
    }
  }

  #[test]
  fn openapi_balances_page_nests_ranking_metadata_under_meta() {
    let doc = serde_json::to_value(ApiDoc::openapi()).expect("openapi should serialize");
    let properties = doc["components"]["schemas"]["BalanceResponse"]["properties"]
      .as_object()
      .expect("balance response properties should exist");
    assert!(properties.contains_key("asOfHeight"));
    assert!(properties.contains_key("totalCount"));
    assert!(properties.contains_key("value"));
    assert!(properties.contains_key("nextCursor"));
    assert!(properties.contains_key("meta"));
    assert!(!properties.contains_key("includedSupply"));
    assert!(!properties.contains_key("supplyDefinition"));

    assert!(properties["meta"]["$ref"]
      .as_str()
      .is_some_and(|value| value.ends_with("/BalancePageMeta")));
  }
}
