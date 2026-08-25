//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! JSON-RPC source module for authenticated node queries.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::rest::rest_err;
use super::RpcSource;
use crate::config::{ResolvedRpcAuth, RpcAuth, RpcConfig, Secret};
use crate::error::{DaemonError, RestErrorKind};
use crate::policy::api::API_TCP_KEEPALIVE_SECS;
use hyphend::wire::msg::RpcDiag;

const JSON_RPC_VERSION: &str = "1.0";

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
  jsonrpc: &'static str,
  id: u64,
  method: &'a str,
  params: Value,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
  result: Value,
  error: Option<RpcErrorObject>,
}

#[derive(Debug, Deserialize)]
struct RpcErrorObject {
  code: i64,
  message: String,
}

/// Typed response for node `getmempoolinfo`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MempoolInfo {
  pub size: u64,
  pub bytes: u64,
  pub usage: u64,
  pub maxmempool: u64,
}

/// One verbose mempool entry keyed by txid.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MempoolTxEntry {
  pub time: i64,
}

/// Typed response for node `getrawmempool true`.
pub type MempoolTxMap = Vec<(String, MempoolTxEntry)>;

/// The node's own count of transactions in the chain, from
/// `getchaintxstats`.
///
/// This is the honest denominator for transaction-side sync progress (task
/// F2). Extrapolating from locally observed density cannot substitute for it:
/// mainnet density rises roughly sixfold toward the tip, so a projection made
/// from the region just indexed is optimistic by about that factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainTxStats {
  /// Transactions in the chain up to `height`.
  pub tx_count: u64,
  /// Height the count is taken at (the node's tip, absent an argument).
  pub height: u32,
}

/// Typed response for node `getnetworkinfo`.
pub type NetworkInfo = Value;

/// Typed response for node `getpeerinfo`.
pub type PeerInfo = Value;

/// Typed response for node governance object listings keyed by hash.
pub type GovernanceObjectMap = Vec<(String, Value)>;

/// Typed response for node current governance votes keyed by vote hash.
pub type GovernanceVoteMap = Vec<(String, String)>;

/// One entry in `protx list ... true`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProTxListEntry {
  pub pro_tx_hash: String,
  #[serde(rename = "type", default)]
  pub validator_type: Option<String>,
  #[serde(default)]
  pub state: Option<ProTxState>,
}

/// Subset of `protx list` live state used for the validator overlay.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProTxState {
  #[serde(default)]
  pub last_paid_height: Option<i32>,
  #[serde(default)]
  pub consecutive_payments: Option<i32>,
  #[serde(rename = "PoSePenalty", default)]
  pub pose_penalty: Option<i32>,
  #[serde(rename = "PoSeRevivedHeight", default)]
  pub pose_revived_height: Option<i32>,
  #[serde(rename = "PoSeBanHeight", default)]
  pub pose_ban_height: Option<i32>,
  #[serde(default)]
  pub revocation_reason: Option<i32>,
  #[serde(default)]
  pub service: Option<String>,
}

/// Typed response for node `getbestchainlock`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BestChainLock {
  pub blockhash: String,
  pub height: u32,
  pub signature: String,
  pub known_block: bool,
  pub hex: String,
}

/// JSON-RPC transport for node methods that are not available over REST.
#[derive(Debug)]
pub struct CoreRpcClient {
  http: Client,
  url: String,
  auth: RpcTransportAuth,
  metrics: RpcMetrics,
}

#[derive(Debug)]
enum RpcTransportAuth {
  CookieFile(PathBuf),
  UserPass { username: String, password: Secret },
}

#[derive(Debug)]
struct RpcMetrics {
  enabled: bool,
  connected: AtomicBool,
  total_calls: AtomicU64,
  total_errors: AtomicU64,
  last_success_epoch: AtomicU64,
  method_counts: Mutex<Vec<(String, u64)>>,
  error_counts: Mutex<Vec<(String, u64)>>,
  auth_kind: RpcAuth,
}

impl RpcMetrics {
  fn new(config: &RpcConfig) -> Result<Self, DaemonError> {
    Ok(Self {
      enabled: config.enabled,
      connected: AtomicBool::new(false),
      total_calls: AtomicU64::new(0),
      total_errors: AtomicU64::new(0),
      last_success_epoch: AtomicU64::new(0),
      method_counts: Mutex::new(Vec::new()),
      error_counts: Mutex::new(Vec::new()),
      auth_kind: config.auth_kind()?,
    })
  }

  fn record_success(&self, method: &str) {
    self.total_calls.fetch_add(1, Ordering::Relaxed);
    self.connected.store(true, Ordering::Relaxed);
    self.bump_counter(&self.method_counts, method);
    let epoch = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    self.last_success_epoch.store(epoch, Ordering::Relaxed);
  }

  fn record_error(&self, category: &str) {
    self.total_calls.fetch_add(1, Ordering::Relaxed);
    self.total_errors.fetch_add(1, Ordering::Relaxed);
    self.connected.store(false, Ordering::Relaxed);
    self.bump_counter(&self.error_counts, category);
  }

  fn snapshot(&self) -> RpcDiag {
    let last_success_epoch = self.last_success_epoch.load(Ordering::Relaxed);
    RpcDiag {
      enabled: self.enabled,
      connected: self.connected.load(Ordering::Relaxed),
      auth_type: Some(match self.auth_kind {
        RpcAuth::Cookie => "cookie".to_owned(),
        RpcAuth::UserPass => "user_pass".to_owned(),
      }),
      total_calls: self.total_calls.load(Ordering::Relaxed),
      total_errors: self.total_errors.load(Ordering::Relaxed),
      last_success_epoch: if last_success_epoch == 0 {
        None
      } else {
        Some(last_success_epoch)
      },
      method_counts: self
        .method_counts
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default(),
      error_counts: self
        .error_counts
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default(),
    }
  }

  fn bump_counter(&self, counters: &Mutex<Vec<(String, u64)>>, key: &str) {
    if let Ok(mut counters) = counters.lock() {
      if let Some((_, count)) = counters.iter_mut().find(|(name, _)| name == key) {
        *count += 1;
      } else {
        counters.push((key.to_owned(), 1));
      }
    }
  }
}

impl CoreRpcClient {
  /// Build a JSON-RPC transport client from the resolved daemon config.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the HTTP client cannot be built.
  pub fn new(config: &RpcConfig) -> Result<Self, DaemonError> {
    let auth = match config.resolve_auth()? {
      ResolvedRpcAuth::CookieFile(path) => RpcTransportAuth::CookieFile(path),
      ResolvedRpcAuth::UserPass { username, password } => {
        RpcTransportAuth::UserPass { username, password }
      }
    };
    let http = Client::builder()
      .timeout(Duration::from_secs(config.timeout_secs))
      .tcp_keepalive(Duration::from_secs(API_TCP_KEEPALIVE_SECS))
      .build()
      .map_err(|e| rest_err("RPC client build", e))?;
    Ok(Self {
      http,
      url: config.url.clone(),
      auth,
      metrics: RpcMetrics::new(config)?,
    })
  }

  /// Execute a single JSON-RPC request and return the raw result value.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when transport, decode, or server-side
  /// JSON-RPC errors occur.
  pub async fn call_value(
    &self,
    id: u64,
    method: &str,
    params: Value,
  ) -> Result<Value, DaemonError> {
    let req = RpcRequest {
      jsonrpc: JSON_RPC_VERSION,
      id,
      method,
      params,
    };
    let cookie = self.auth_header()?;
    let resp = self
      .http
      .post(&self.url)
      .header(reqwest::header::AUTHORIZATION, cookie)
      .json(&req)
      .send()
      .await
      .map_err(|e| {
        self.metrics.record_error("transport");
        rest_err("RPC request", e)
      })?;

    if !resp.status().is_success() {
      self
        .metrics
        .record_error(&format!("http_{}", resp.status().as_u16()));
      return Err(DaemonError::RestSource {
        message: format!("RPC HTTP {}", resp.status()),
        kind: RestErrorKind::Http(resp.status().as_u16()),
      });
    }

    let body: RpcResponse = resp.json().await.map_err(|e| {
      self.metrics.record_error("decode");
      DaemonError::RestSource {
        message: format!("RPC JSON parse: {e}"),
        kind: RestErrorKind::Other,
      }
    })?;

    if let Some(error) = body.error {
      self.metrics.record_error(&format!("rpc_{}", error.code));
      return Err(DaemonError::RestSource {
        message: format!("RPC {} {}", error.code, error.message),
        kind: RestErrorKind::Other,
      });
    }

    self.metrics.record_success(method);
    Ok(body.result)
  }

  /// Return a typed JSON-RPC diagnostic snapshot.
  pub fn diag(&self) -> RpcDiag {
    self.metrics.snapshot()
  }

  /// Read the current mempool summary from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request fails or the
  /// response shape is not compatible with `getmempoolinfo`.
  pub async fn get_mempool_info(&self) -> Result<MempoolInfo, DaemonError> {
    let value = self
      .call_value(0, "getmempoolinfo", Value::Array(vec![]))
      .await?;
    decode_mempool_info(value)
  }

  /// Read the current verbose mempool snapshot keyed by txid.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request fails or the
  /// response shape is not compatible with `getrawmempool true`.
  pub async fn get_raw_mempool(&self) -> Result<MempoolTxMap, DaemonError> {
    let value = self
      .call_value(0, "getrawmempool", Value::Array(vec![Value::Bool(true)]))
      .await?;
    decode_raw_mempool(value)
  }

  /// Read one raw serialized transaction by txid.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request fails or the
  /// response shape is not compatible with `getrawtransaction false`.
  pub async fn get_raw_transaction(&self, txid: &str) -> Result<Vec<u8>, DaemonError> {
    let value = self
      .call_value(
        0,
        "getrawtransaction",
        Value::Array(vec![Value::String(txid.to_owned()), Value::Bool(false)]),
      )
      .await?;
    decode_raw_transaction(value)
  }

  /// Read the node's chain-wide transaction count.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request fails or the
  /// response does not carry `txcount` and `window_final_block_height`.
  pub async fn get_chain_tx_stats(&self) -> Result<ChainTxStats, DaemonError> {
    let value = self
      .call_value(0, "getchaintxstats", Value::Array(vec![]))
      .await?;
    parse_chain_tx_stats(&value)
  }

  /// Read the current network summary from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request fails.
  pub async fn get_network_info(&self) -> Result<NetworkInfo, DaemonError> {
    self
      .call_value(0, "getnetworkinfo", Value::Array(vec![]))
      .await
  }

  /// Read the current peer list from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request fails.
  pub async fn get_peer_info(&self) -> Result<PeerInfo, DaemonError> {
    self
      .call_value(0, "getpeerinfo", Value::Array(vec![]))
      .await
  }

  /// Read the current best ChainLock from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request or decode fails.
  pub async fn get_best_chainlock(&self) -> Result<BestChainLock, DaemonError> {
    let value = self
      .call_value(0, "getbestchainlock", Value::Array(vec![]))
      .await?;
    decode_best_chainlock(value)
  }

  /// Read current governance proposals from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request or decode fails.
  pub async fn list_governance_proposals(&self) -> Result<GovernanceObjectMap, DaemonError> {
    let value = self
      .call_value(
        0,
        "gobject",
        Value::Array(vec![
          Value::String("list".to_owned()),
          Value::String("all".to_owned()),
          Value::String("proposals".to_owned()),
        ]),
      )
      .await?;
    decode_governance_objects(value)
  }

  /// Read one governance object by hash from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request or decode fails.
  pub async fn get_governance_object(&self, object_hash: &str) -> Result<Value, DaemonError> {
    self
      .call_value(
        0,
        "gobject",
        Value::Array(vec![
          Value::String("get".to_owned()),
          Value::String(object_hash.to_owned()),
        ]),
      )
      .await
  }

  /// Read current governance votes for one proposal from the node.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request or decode fails.
  pub async fn get_governance_votes(
    &self,
    object_hash: &str,
  ) -> Result<GovernanceVoteMap, DaemonError> {
    let value = self
      .call_value(
        0,
        "gobject",
        Value::Array(vec![
          Value::String("getcurrentvotes".to_owned()),
          Value::String(object_hash.to_owned()),
        ]),
      )
      .await?;
    decode_governance_votes(value)
  }

  /// Read the latest registered deterministic masternode list.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request or decode fails.
  pub async fn get_protx_list_registered(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
    let value = self
      .call_value(
        0,
        "protx",
        Value::Array(vec![
          Value::String("list".to_owned()),
          Value::String("registered".to_owned()),
          Value::Bool(true),
        ]),
      )
      .await?;
    decode_protx_list(value)
  }

  /// Read the latest valid deterministic masternode list.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the RPC request or decode fails.
  pub async fn get_protx_list_valid(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
    let value = self
      .call_value(
        0,
        "protx",
        Value::Array(vec![
          Value::String("list".to_owned()),
          Value::String("valid".to_owned()),
          Value::Bool(true),
        ]),
      )
      .await?;
    decode_protx_list(value)
  }

  fn auth_header(&self) -> Result<String, DaemonError> {
    match &self.auth {
      RpcTransportAuth::CookieFile(path) => {
        let cookie = fs::read_to_string(path).map_err(|e| DaemonError::Io {
          context: "read node RPC cookie",
          source: e,
        })?;
        Ok(format!("Basic {}", encode_basic_token(cookie.trim_end())))
      }
      RpcTransportAuth::UserPass { username, password } => Ok(format!(
        "Basic {}",
        encode_basic_token(&format!("{username}:{}", password.expose()))
      )),
    }
  }
}

/// Pull the two fields transaction-side progress needs out of a
/// `getchaintxstats` response.
fn parse_chain_tx_stats(value: &Value) -> Result<ChainTxStats, DaemonError> {
  let shape = |field: &str| DaemonError::RestSource {
    message: format!("getchaintxstats: missing or non-numeric `{field}`"),
    kind: RestErrorKind::Other,
  };
  let tx_count = value
    .get("txcount")
    .and_then(Value::as_u64)
    .ok_or_else(|| shape("txcount"))?;
  let height = value
    .get("window_final_block_height")
    .and_then(Value::as_u64)
    .and_then(|h| u32::try_from(h).ok())
    .ok_or_else(|| shape("window_final_block_height"))?;
  Ok(ChainTxStats { tx_count, height })
}

#[async_trait::async_trait]

impl RpcSource for CoreRpcClient {
  async fn get_mempool_info(&self) -> Result<MempoolInfo, DaemonError> {
    CoreRpcClient::get_mempool_info(self).await
  }

  async fn get_raw_mempool(&self) -> Result<MempoolTxMap, DaemonError> {
    CoreRpcClient::get_raw_mempool(self).await
  }

  async fn get_raw_transaction(&self, txid: &str) -> Result<Vec<u8>, DaemonError> {
    CoreRpcClient::get_raw_transaction(self, txid).await
  }

  async fn get_chain_tx_stats(&self) -> Result<ChainTxStats, DaemonError> {
    CoreRpcClient::get_chain_tx_stats(self).await
  }

  async fn get_network_info(&self) -> Result<NetworkInfo, DaemonError> {
    CoreRpcClient::get_network_info(self).await
  }

  async fn get_peer_info(&self) -> Result<PeerInfo, DaemonError> {
    CoreRpcClient::get_peer_info(self).await
  }

  async fn get_best_chainlock(&self) -> Result<BestChainLock, DaemonError> {
    CoreRpcClient::get_best_chainlock(self).await
  }

  async fn get_protx_list_registered(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
    CoreRpcClient::get_protx_list_registered(self).await
  }

  async fn get_protx_list_valid(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
    CoreRpcClient::get_protx_list_valid(self).await
  }

  fn diag(&self) -> RpcDiag {
    CoreRpcClient::diag(self)
  }
}

fn encode_basic_token(value: &str) -> String {
  const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

  let bytes = value.as_bytes();
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  let mut chunks = bytes.chunks_exact(3);

  for chunk in &mut chunks {
    let a = chunk[0];
    let b = chunk[1];
    let c = chunk[2];
    out.push(TABLE[(a >> 2) as usize] as char);
    out.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
    out.push(TABLE[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char);
    out.push(TABLE[(c & 0x3f) as usize] as char);
  }

  let rem = chunks.remainder();
  if !rem.is_empty() {
    let a = rem[0];
    out.push(TABLE[(a >> 2) as usize] as char);
    if rem.len() == 1 {
      out.push(TABLE[((a & 0x03) << 4) as usize] as char);
      out.push('=');
      out.push('=');
    } else {
      let b = rem[1];
      out.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
      out.push(TABLE[((b & 0x0f) << 2) as usize] as char);
      out.push('=');
    }
  }

  out
}

fn decode_mempool_info(value: Value) -> Result<MempoolInfo, DaemonError> {
  serde_json::from_value(value).map_err(|e| DaemonError::RestSource {
    message: format!("RPC getmempoolinfo decode: {e}"),
    kind: RestErrorKind::Other,
  })
}

fn decode_raw_mempool(value: Value) -> Result<MempoolTxMap, DaemonError> {
  let object = value.as_object().ok_or_else(|| DaemonError::RestSource {
    message: "RPC getrawmempool decode: expected object".to_owned(),
    kind: RestErrorKind::Other,
  })?;
  object
    .iter()
    .map(|(txid, entry)| {
      serde_json::from_value(entry.clone())
        .map(|entry| (txid.clone(), entry))
        .map_err(|e| DaemonError::RestSource {
          message: format!("RPC getrawmempool decode: {e}"),
          kind: RestErrorKind::Other,
        })
    })
    .collect()
}

fn decode_raw_transaction(value: Value) -> Result<Vec<u8>, DaemonError> {
  let raw_tx_hex = value.as_str().ok_or_else(|| DaemonError::RestSource {
    message: "RPC getrawtransaction decode: expected hex string".to_owned(),
    kind: RestErrorKind::Other,
  })?;
  hex::decode(raw_tx_hex).map_err(|e| DaemonError::RestSource {
    message: format!("RPC getrawtransaction decode: {e}"),
    kind: RestErrorKind::Other,
  })
}

fn decode_protx_list(value: Value) -> Result<Vec<ProTxListEntry>, DaemonError> {
  serde_json::from_value(value).map_err(|e| DaemonError::RestSource {
    message: format!("RPC protx list decode: {e}"),
    kind: RestErrorKind::Other,
  })
}

fn decode_best_chainlock(value: Value) -> Result<BestChainLock, DaemonError> {
  serde_json::from_value(value).map_err(|e| DaemonError::RestSource {
    message: format!("RPC getbestchainlock decode: {e}"),
    kind: RestErrorKind::Other,
  })
}

fn decode_governance_objects(value: Value) -> Result<GovernanceObjectMap, DaemonError> {
  let object = value.as_object().ok_or_else(|| DaemonError::RestSource {
    message: "RPC gobject list decode: expected object".to_owned(),
    kind: RestErrorKind::Other,
  })?;
  Ok(
    object
      .iter()
      .map(|(hash, value)| (hash.clone(), value.clone()))
      .collect(),
  )
}

fn decode_governance_votes(value: Value) -> Result<GovernanceVoteMap, DaemonError> {
  let object = value.as_object().ok_or_else(|| DaemonError::RestSource {
    message: "RPC gobject getcurrentvotes decode: expected object".to_owned(),
    kind: RestErrorKind::Other,
  })?;
  object
    .iter()
    .map(|(hash, value)| {
      value
        .as_str()
        .map(|vote| (hash.clone(), vote.to_owned()))
        .ok_or_else(|| DaemonError::RestSource {
          message: "RPC governance vote decode: expected string vote".to_owned(),
          kind: RestErrorKind::Other,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use std::fs;
  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  use super::{
    decode_best_chainlock, decode_mempool_info, decode_raw_mempool, decode_raw_transaction,
    encode_basic_token, parse_chain_tx_stats, BestChainLock, CoreRpcClient, MempoolTxEntry,
    RpcMetrics,
  };
  use crate::config::{RpcAuth, RpcConfig};

  /// F2: the chain-wide transaction count is the only honest denominator for
  /// transaction-side progress, so its two fields are read strictly.
  #[test]
  fn chain_tx_stats_reads_the_count_and_its_height() {
    // Shape as mainnet answers it.
    let value = serde_json::json!({
      "time": 1_787_127_692i64,
      "txcount": 70_556_609u64,
      "window_final_block_height": 2_524_355u64,
      "window_tx_count": 358_976u64,
      "txrate": 0.131_741_902_193_998
    });
    let stats = parse_chain_tx_stats(&value);
    assert!(stats.is_ok(), "well-formed getchaintxstats must parse");
    if let Ok(stats) = stats {
      assert_eq!(stats.tx_count, 70_556_609);
      assert_eq!(stats.height, 2_524_355);
    }

    // A missing or wrongly-typed field is an error, not a zero: a zero would
    // silently read as "chain has no transactions" and show 100% progress.
    for bad in [
      serde_json::json!({ "window_final_block_height": 1u64 }),
      serde_json::json!({ "txcount": "70556609", "window_final_block_height": 1u64 }),
      serde_json::json!({ "txcount": 1u64 }),
      serde_json::json!({}),
    ] {
      assert!(
        parse_chain_tx_stats(&bad).is_err(),
        "malformed getchaintxstats must not parse: {bad}"
      );
    }
  }

  fn unique_temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("time should move forward")
      .as_nanos();
    std::env::temp_dir().join(format!("hyphen-{name}-{nanos}"))
  }

  #[test]
  fn basic_token_matches_cookie_auth_shape() {
    assert_eq!(
      encode_basic_token("__cookie__:secret"),
      "X19jb29raWVfXzpzZWNyZXQ="
    );
  }

  #[test]
  fn cookie_auth_header_reads_latest_cookie_contents() {
    let path = unique_temp_path("rpc-cookie");
    fs::write(&path, "__cookie__:first\n").expect("cookie write should succeed");

    let cfg = RpcConfig {
      cookie_file: Some(path.clone()),
      ..RpcConfig::default()
    };
    let client = CoreRpcClient::new(&cfg).expect("client should build");
    assert_eq!(
      client.auth_header().expect("cookie header should build"),
      "Basic X19jb29raWVfXzpmaXJzdA=="
    );

    fs::write(&path, "__cookie__:second\n").expect("cookie rewrite should succeed");
    assert_eq!(
      client.auth_header().expect("cookie header should rebuild"),
      "Basic X19jb29raWVfXzpzZWNvbmQ="
    );

    fs::remove_file(path).expect("cookie cleanup should succeed");
  }

  #[test]
  fn userpass_auth_header_encodes_basic_credentials() {
    let cfg = RpcConfig {
      username: Some("alice".to_owned()),
      password: Some("secret".to_owned().into()),
      ..RpcConfig::default()
    };
    let client = CoreRpcClient::new(&cfg).expect("client should build");
    assert_eq!(
      client.auth_header().expect("userpass header should build"),
      "Basic YWxpY2U6c2VjcmV0"
    );
  }

  #[test]
  fn rpc_diag_tracks_method_and_error_counters() {
    let metrics = RpcMetrics::new(&RpcConfig {
      username: Some("alice".to_owned()),
      password: Some("secret".to_owned().into()),
      ..RpcConfig::default()
    })
    .expect("metrics should build");

    metrics.record_success("getnetworkinfo");
    metrics.record_success("getnetworkinfo");
    metrics.record_error("transport");

    let diag = metrics.snapshot();
    assert!(diag.enabled == false);
    assert!(diag.connected == false);
    assert_eq!(diag.auth_type, Some("user_pass".to_owned()));
    assert_eq!(diag.total_calls, 3);
    assert_eq!(diag.total_errors, 1);
    assert_eq!(diag.method_counts, vec![("getnetworkinfo".to_owned(), 2)]);
    assert_eq!(diag.error_counts, vec![("transport".to_owned(), 1)]);
    assert!(matches!(metrics.auth_kind, RpcAuth::UserPass));
  }

  #[test]
  fn decode_mempool_info_reads_expected_fields() {
    let info = decode_mempool_info(serde_json::json!({
      "size": 12,
      "bytes": 345,
      "usage": 678,
      "maxmempool": 999
    }))
    .expect("mempool info should decode");
    assert_eq!(
      info,
      super::MempoolInfo {
        size: 12,
        bytes: 345,
        usage: 678,
        maxmempool: 999,
      }
    );
  }

  #[test]
  fn decode_best_chainlock_reads_expected_fields() {
    let chainlock = decode_best_chainlock(serde_json::json!({
      "blockhash": "11".repeat(32),
      "height": 42,
      "signature": "22".repeat(96),
      "known_block": true,
      "hex": "33".repeat(128),
    }))
    .expect("best chainlock should decode");

    assert_eq!(
      chainlock,
      BestChainLock {
        blockhash: "11".repeat(32),
        height: 42,
        signature: "22".repeat(96),
        known_block: true,
        hex: "33".repeat(128),
      }
    );
  }

  #[test]
  fn decode_raw_mempool_reads_expected_fields() {
    let entries = decode_raw_mempool(serde_json::json!({
      "11".repeat(32): { "time": 123 },
      "22".repeat(32): { "time": 456 }
    }))
    .expect("raw mempool should decode");

    assert_eq!(
      entries,
      vec![
        ("11".repeat(32), MempoolTxEntry { time: 123 }),
        ("22".repeat(32), MempoolTxEntry { time: 456 }),
      ]
    );
  }

  #[test]
  fn decode_raw_transaction_reads_expected_hex_payload() {
    let raw_tx =
      decode_raw_transaction(serde_json::json!("deadbeef")).expect("raw transaction should decode");
    assert_eq!(raw_tx, vec![0xde, 0xad, 0xbe, 0xef]);
  }
}
