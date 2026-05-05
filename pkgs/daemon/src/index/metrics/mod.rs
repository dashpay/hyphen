//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Cumulative block metrics index.

pub(crate) mod ipc;
#[cfg(feature = "tests")]
pub mod rest;
#[cfg(not(feature = "tests"))]
pub(crate) mod rest;
pub(crate) mod util;

use std::sync::Mutex;

use dash_script::ScriptKind;

use crate::error::DaemonError;
pub(crate) use crate::sql::index::metrics as sql;

use super::{BlockCtx, IndexOps};

/// Cached cumulative values from the last processed block.
/// (height, circulating_supply, tx_count, utxos_spendable,
///  utxos_unspendable, blockchain_size)
type MetricsCache = (u32, i64, i64, i64, i64, i64);
type PreviousMetrics = (i64, i64, i64, i64, i64);

#[derive(Debug)]
pub struct MetricsIndex {
  cache: Mutex<Option<MetricsCache>>,
  block_time_secs: f64,
}

impl MetricsIndex {
  pub fn new(block_time_secs: f64) -> Self {
    Self {
      cache: Mutex::new(None),
      block_time_secs,
    }
  }
}

#[async_trait::async_trait]
impl IndexOps for MetricsIndex {
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    sql::delete_block_metrics(tx, height).await?;
    *self
      .cache
      .lock()
      .map_err(|e| DaemonError::Index(format!("cache lock: {e}")))? = None;
    Ok(())
  }

  fn reset_cache(&self) {
    if let Ok(mut c) = self.cache.lock() {
      *c = None;
    }
  }
}

impl MetricsIndex {
  pub(crate) async fn plan_connect(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    block: &BlockCtx,
  ) -> Result<MetricsBlockPlan, DaemonError> {
    let previous_metrics = match self.cached_metrics(block.height())? {
      Some(metrics) => metrics,
      None => sql::query_prev_metrics(tx, block.height()).await?,
    };
    Ok(MetricsBlockPlan::build(
      block.block(),
      block.raw(),
      block.height(),
      previous_metrics,
      self.block_time_secs,
    ))
  }

  pub(crate) async fn apply_plan(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &MetricsBlockPlan,
  ) -> Result<(), DaemonError> {
    plan.apply(tx).await?;
    self.store_cache(plan.next_cache())?;
    Ok(())
  }

  fn cached_metrics(&self, height: u32) -> Result<Option<PreviousMetrics>, DaemonError> {
    if height == 0 {
      return Ok(Some((0, 0, 0, 0, 0)));
    }

    let cached = self
      .cache
      .lock()
      .map_err(|e| DaemonError::Index(format!("cache lock: {e}")))?;
    Ok(cached.and_then(
      |(h, supply, tx_count, spendable, unspendable, chain_size)| {
        if h == height - 1 {
          Some((supply, tx_count, spendable, unspendable, chain_size))
        } else {
          None
        }
      },
    ))
  }

  fn store_cache(&self, next_cache: MetricsCache) -> Result<(), DaemonError> {
    *self
      .cache
      .lock()
      .map_err(|e| DaemonError::Index(format!("cache lock: {e}")))? = Some(next_cache);
    Ok(())
  }
}

pub(crate) struct MetricsBlockPlan {
  height: u32,
  tx_count: i64,
  utxos_spendable: i64,
  utxos_unspendable: i64,
  blockchain_size: i64,
  credit_pool: Option<i64>,
  circulating_supply: i64,
  difficulty: f64,
  hashrate: f64,
}

impl MetricsBlockPlan {
  fn build(
    block: &dash_primitives::Block,
    raw: &[u8],
    height: u32,
    previous_metrics: PreviousMetrics,
    block_time_secs: f64,
  ) -> Self {
    let mut spendable_created = 0;
    let mut unspendable_created = 0;
    let mut inputs_spent = 0;
    let mut coinbase_value = 0;

    for (index, transaction) in block.transactions.iter().enumerate() {
      for output in &transaction.outputs {
        match dash_script::classify(output.script_pubkey.as_bytes()) {
          ScriptKind::P2pkh | ScriptKind::P2sh | ScriptKind::P2pk => spendable_created += 1,
          ScriptKind::OpReturn | ScriptKind::Unknown => unspendable_created += 1,
        }
      }

      if index == 0 && util::is_coinbase_tx(transaction) {
        coinbase_value = transaction
          .outputs
          .iter()
          .map(|output| output.value.to_sat() as i64)
          .sum();
      } else {
        for input in &transaction.inputs {
          if !input.prevout.hash.is_null() {
            inputs_spent += 1;
          }
        }
      }
    }

    let (prev_supply, prev_tx_count, prev_spendable, prev_unspendable, prev_chain_size) =
      previous_metrics;

    let tx_count = prev_tx_count + block.transactions.len() as i64;
    let utxos_spendable = prev_spendable + spendable_created - inputs_spent;
    let utxos_unspendable = prev_unspendable + unspendable_created;
    let blockchain_size = prev_chain_size + raw.len() as i64;
    let circulating_supply = prev_supply + coinbase_value;
    let credit_pool = util::extract_credit_pool(block);
    let difficulty = crate::consensus::bits_to_difficulty(block.header.bits);
    let hashrate = util::hashrate_from_difficulty(difficulty, block_time_secs);

    Self {
      height,
      tx_count,
      utxos_spendable,
      utxos_unspendable,
      blockchain_size,
      credit_pool,
      circulating_supply,
      difficulty,
      hashrate,
    }
  }

  async fn apply(&self, tx: &mut crate::sql::SqlTx<'_>) -> Result<(), DaemonError> {
    sql::insert_block_metrics(
      tx,
      self.height,
      self.tx_count,
      self.utxos_spendable,
      self.utxos_unspendable,
      self.blockchain_size,
      self.credit_pool,
      self.circulating_supply,
      self.difficulty,
      self.hashrate,
    )
    .await
  }

  fn next_cache(&self) -> MetricsCache {
    (
      self.height,
      self.circulating_supply,
      self.tx_count,
      self.utxos_spendable,
      self.utxos_unspendable,
      self.blockchain_size,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use dash_primitives::{
    Block, BlockHash, BlockHeader, MerkleRoot, OutPoint, Script, Transaction, TxHash, TxIn, TxOut, TxType,
  };

  fn test_block(
    first_tx_type: TxType,
    first_input_hash: [u8; 32],
    first_input_index: u32,
  ) -> Block {
    Block {
      header: BlockHeader {
        version: 4,
        prev_hash: BlockHash::from_bytes([0u8; 32]),
        merkle_root: MerkleRoot::from_bytes([1u8; 32]),
        time: 1,
        bits: 0x1d00ffff,
        nonce: 0,
      },
      transactions: vec![Transaction {
        version: 3,
        tx_type: first_tx_type,
        inputs: vec![TxIn {
          prevout: OutPoint {
            hash: TxHash::from_bytes(first_input_hash),
            index: first_input_index,
          },
          script_sig: Script(Vec::new()),
          sequence: u32::MAX,
        }],
        outputs: vec![TxOut {
          value: bitcoin_units::Amount::from_sat_u32(500_000_000),
          script_pubkey: Script(vec![0x51]),
        }],
        lock_time: 0,
        extra_payload: Vec::new(),
      }],
    }
  }

  #[test]
  fn circulating_supply_uses_structural_coinbase_before_special_tx_activation() {
    let block = test_block(TxType::Spend, [0u8; 32], u32::MAX);
    let plan = MetricsBlockPlan::build(&block, &[0u8; 80], 1, (0, 0, 0, 0, 0), 150.0);
    assert_eq!(plan.circulating_supply, 500_000_000);
  }

  #[test]
  fn circulating_supply_ignores_non_coinbase_first_transaction() {
    let block = test_block(TxType::CoinbaseCommitment, [2u8; 32], 0);
    let plan = MetricsBlockPlan::build(&block, &[0u8; 80], 1, (0, 0, 0, 0, 0), 150.0);
    assert_eq!(plan.circulating_supply, 0);
  }
}
