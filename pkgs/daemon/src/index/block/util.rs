//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block utility functions (difficulty, header extraction).

use num_bigint::BigUint;

use dash_primitives::payload::SpecialPayload;
use dash_primitives::Block;

/// Coinbase transaction fields extracted from the block.
#[derive(Debug, Default)]
pub struct CbTxFields {
  pub height: Option<i32>,
  pub merkle_root: Option<Vec<u8>>,
  pub merkle_flags: Option<i32>,
  pub best_cl_height: Option<i32>,
  pub best_cl_sig: Option<Vec<u8>>,
  pub credit_pool: Option<i64>,
}

/// Extract coinbase transaction fields from the first transaction.
pub fn extract_cbtx_fields(block: &Block, _height: u32) -> CbTxFields {
  let tx = match block.transactions.first() {
    Some(t) => t,
    None => return CbTxFields::default(),
  };

  match tx.decode_payload() {
    Some(Ok(SpecialPayload::CoinbaseCommitment(cbtx))) => CbTxFields {
      height: Some(cbtx.height.to_u32() as i32),
      merkle_root: Some(cbtx.merkle_root_mn_list.as_bytes().to_vec()),
      merkle_flags: Some(cbtx.version as i32),
      best_cl_height: cbtx
        .best_cl_height_diff
        .map(|diff| (cbtx.height.to_u32() as i64 - diff as i64) as i32),
      best_cl_sig: cbtx
        .best_cl_signature
        .as_ref()
        .map(|s| s.0.to_vec()),
      credit_pool: cbtx.credit_pool_balance,
    },
    _ => CbTxFields::default(),
  }
}

/// Compute the proof-of-work for a given compact target (nBits).
pub fn block_proof(bits: u32) -> BigUint {
  let target = compact_to_target(bits);
  if target.eq(&BigUint::ZERO) {
    return BigUint::ZERO;
  }
  let max_256 = (BigUint::from(1u32) << 256) - BigUint::from(1u32);
  (&max_256 - &target) / (&target + BigUint::from(1u32)) + BigUint::from(1u32)
}

/// Expand compact "bits" to a full 256-bit target.
fn compact_to_target(bits: u32) -> BigUint {
  let exp = (bits >> 24) as usize;
  let mantissa = bits & 0x007f_ffff;
  let negative = (bits & 0x0080_0000) != 0;
  if negative || mantissa == 0 || exp == 0 {
    return BigUint::ZERO;
  }
  let mut target = BigUint::from(mantissa);
  if exp > 3 {
    target <<= 8 * (exp - 3);
  } else {
    target >>= 8 * (3 - exp);
  }
  target
}
