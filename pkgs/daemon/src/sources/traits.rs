//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Source capability traits for durable and live node inputs.

use crate::error::DaemonError;
use async_trait::async_trait;
use hyphend::wire::msg::{CoreInfo, RestClientStats};

use super::rpc::{
  BestChainLock, ChainTxStats, MempoolInfo, MempoolTxMap, NetworkInfo, PeerInfo, ProTxListEntry,
};
use super::zmq::ZmqEvent;

/// Async interface for fetching canonical block data from node REST.
#[async_trait]
pub trait RestSource: Send + Sync + std::fmt::Debug {
  /// Lightweight connectivity check.
  async fn ping(&self) -> Result<(), DaemonError> {
    self.block_count().await.map(|_| ())
  }

  /// Return cumulative outbound REST client metrics, if available.
  fn rest_stats(&self) -> Option<RestClientStats> {
    None
  }

  /// Return core node version metadata, if available.
  fn core_info(&self) -> Option<CoreInfo> {
    None
  }
  /// Current chain height (block count - 1).
  async fn block_count(&self) -> Result<u32, DaemonError>;

  /// Hash for a given block height.
  async fn block_hash(&self, height: u32) -> Result<String, DaemonError>;

  /// Raw serialized block bytes by hash.
  async fn raw_block(&self, hash: &str) -> Result<Vec<u8>, DaemonError>;

  /// Fetch (hash, raw_block) pairs for a batch of heights.
  async fn fetch_blocks_batch(
    &self,
    heights: &[u32],
  ) -> Result<Vec<(String, Vec<u8>)>, DaemonError>;
}

/// Async interface for authenticated JSON-RPC queries against the node.
#[async_trait]
pub trait RpcSource: Send + Sync + std::fmt::Debug {
  /// Read the current mempool summary.
  async fn get_mempool_info(&self) -> Result<MempoolInfo, DaemonError>;

  /// Read the current verbose mempool transaction snapshot keyed by txid.
  async fn get_raw_mempool(&self) -> Result<MempoolTxMap, DaemonError>;

  /// Read one raw serialized transaction from the node mempool or chain view.
  async fn get_raw_transaction(&self, txid: &str) -> Result<Vec<u8>, DaemonError>;

  /// Read the node's chain-wide transaction count, the denominator for
  /// transaction-side sync progress (task F2).
  async fn get_chain_tx_stats(&self) -> Result<ChainTxStats, DaemonError>;

  /// Read the current network summary.
  async fn get_network_info(&self) -> Result<NetworkInfo, DaemonError>;

  /// Read the current peer list.
  async fn get_peer_info(&self) -> Result<PeerInfo, DaemonError>;

  /// Read the latest known ChainLock snapshot.
  async fn get_best_chainlock(&self) -> Result<BestChainLock, DaemonError>;

  /// Read the latest registered deterministic masternode list.
  async fn get_protx_list_registered(&self) -> Result<Vec<ProTxListEntry>, DaemonError>;

  /// Read the latest valid deterministic masternode list.
  async fn get_protx_list_valid(&self) -> Result<Vec<ProTxListEntry>, DaemonError>;

  /// Return cumulative JSON-RPC diagnostics for snapshot and health surfaces.
  fn diag(&self) -> hyphend::wire::msg::RpcDiag;
}

/// Async interface for live event intake such as ZMQ subscriptions.
#[async_trait]
pub trait ZmqSource: Send + Sync + std::fmt::Debug {
  /// Receive and decode the next live event.
  async fn recv_event(&self) -> Result<ZmqEvent, DaemonError>;

  /// Return cumulative ZMQ diagnostics for snapshot and health surfaces.
  fn diag(&self) -> hyphend::wire::msg::ZmqDiag;
}
