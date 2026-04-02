//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Helper functions for metrics computation.

use dash_primitives::tx::transaction::Transaction;
use dash_primitives::types::tx::TxType;

/// Return whether a transaction is structurally a coinbase transaction.
pub fn is_coinbase_tx(transaction: &Transaction) -> bool {
  transaction.inputs.len() == 1
    && transaction.inputs[0].prevout.hash.0 == [0u8; 32]
    && transaction.inputs[0].prevout.index == u32::MAX
}

/// Extract the credit pool balance from the coinbase payload.
pub fn extract_credit_pool(block: &dash_primitives::block::full::Block) -> Option<i64> {
  let coinbase = block.transactions.first()?;
  if coinbase.tx_type != TxType::Coinbase {
    return None;
  }
  let cbtx = coinbase
    .payload::<dash_primitives::tx::coinbase::CCbTx>()
    .ok()?;
  cbtx.credit_pool_balance
}

/// Estimate network hashrate from difficulty and target block time.
pub fn hashrate_from_difficulty(difficulty: f64, block_time_secs: f64) -> f64 {
  difficulty * 4_294_967_296.0 / block_time_secs
}

#[cfg(test)]
mod tests {
  use super::*;
  use dash_primitives::tx::transaction::Transaction;
  use dash_primitives::types::common::{Script, TxHash};
  use dash_primitives::types::tx::{OutPoint, TxIn, TxOut, TxType};

  fn tx_with_prevout(hash: [u8; 32], index: u32, tx_type: TxType) -> Transaction {
    Transaction {
      version: 3,
      tx_type,
      inputs: vec![TxIn {
        prevout: OutPoint {
          hash: TxHash(hash),
          index,
        },
        script_sig: Script(Vec::new()),
        sequence: u32::MAX,
      }],
      outputs: vec![TxOut {
        value: 50 * 100_000_000,
        script_pubkey: Script(Vec::new()),
      }],
      lock_time: 0,
      extra_payload: Vec::new(),
    }
  }

  #[test]
  fn structural_coinbase_is_detected_for_normal_tx_type() {
    let tx = tx_with_prevout([0u8; 32], u32::MAX, TxType::Normal);
    assert!(is_coinbase_tx(&tx));
  }

  #[test]
  fn non_coinbase_prevout_is_rejected() {
    let tx = tx_with_prevout([1u8; 32], 0, TxType::Coinbase);
    assert!(!is_coinbase_tx(&tx));
  }
}
