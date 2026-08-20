//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The daemon's chain-decode seam for `hyphen-db`.
//!
//! [`HyDbBlockSource`] implements [`hyphen_db::HyDbDataSrc`]: it turns raw
//! block bytes into the storage crate's structured fact family. Decoding is
//! **pure** with respect to the bytes — no I/O, no node calls — so the crate
//! can replay it during reorg re-derivation and get identical facts.

use dash_primitives::payload::SpecialPayload;
use dash_primitives::{Transaction, TxType};

use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbCbFields, HyDbError, HyDbHash,
  HyDbHeader, HyDbOutpoint, HyDbPayload, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut, HyDbTxType,
  HyDbVlEvent, HyDbVlRegister, HyDbVlRevoke, HyDbVlType, HyDbVlUpdRegistrar, HyDbVlUpdService,
};

use crate::tx::{ValidatorEffect, ValidatorInsert};

/// Decodes raw Dash block bytes into `hyphen-db` facts. Holds the chain's
/// address-version bytes, needed to derive output/validator addresses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HyDbBlockSource {
  p2pkh_version: u8,
  p2sh_version: u8,
}

impl HyDbBlockSource {
  pub(crate) fn new(p2pkh_version: u8, p2sh_version: u8) -> Self {
    Self {
      p2pkh_version,
      p2sh_version,
    }
  }

  /// The 80-byte header alone.
  ///
  /// A block's header is a fixed-size prefix, so reading a header field costs
  /// nothing more than those 80 bytes. Decoding the whole `Block` to reach one
  /// walks every input and output and heap-allocates a vector per script,
  /// which for [`Self::identify`] — a `prev_hash` lookup — was the entire cost
  /// of the call.
  fn decode_header(raw: &[u8]) -> HyDbResult<dash_primitives::block_header::BlockHeader> {
    let head = raw
      .get(..80)
      .ok_or_else(|| HyDbError::Derive("decode header: block is under 80 bytes".to_owned()))?;
    bitcoin_consensus_encoding::decode_from_slice::<dash_primitives::block_header::BlockHeader>(
      head,
    )
    .map_err(|e| HyDbError::Derive(format!("decode header: {e}")))
  }

  fn decode_block(raw: &[u8]) -> HyDbResult<dash_primitives::Block> {
    bitcoin_consensus_encoding::decode_from_slice::<dash_primitives::Block>(raw)
      .map_err(|e| HyDbError::Derive(format!("decode block: {e}")))
  }

  fn block_hash(raw: &[u8]) -> HyDbResult<HyDbHash> {
    crate::consensus::block_hash(raw)
      .map(|h| HyDbHash(h.to_bytes()))
      .map_err(|e| HyDbError::Derive(format!("hash block: {e}")))
  }
}

impl hyphen_db::HyDbDataSrc for HyDbBlockSource {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    // Header only: this answers "what is this block and what is its parent",
    // and both live in the fixed 80-byte prefix.
    let header = Self::decode_header(raw)?;
    Ok(HyDbBlockId {
      hash: Self::block_hash(raw)?,
      prev_hash: HyDbHash(header.prev_hash.to_bytes()),
    })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    // Still two passes over the transactions: `decode_block` decodes them, and
    // `tx_byte_ranges` decodes them again to record where each one ends. One
    // pass could yield both, but only from inside `dash-primitives` — the
    // daemon is barred from wire-level parsing by
    // `architecture::runtime_primitive_work_stays_inside_primitives`, and that
    // boundary is worth more than the pass. It needs a primitives-side
    // `tx_byte_ranges` that returns the decoded transactions alongside the
    // spans it already computes.
    let block = Self::decode_block(raw)?;
    let ranges = dash_primitives::block::tx_byte_ranges(raw)
      .map_err(|e| HyDbError::Derive(format!("scan txs: {e}")))?;
    if ranges.len() != block.transactions.len() {
      return Err(HyDbError::Derive(format!(
        "tx range count {} != decoded tx count {}",
        ranges.len(),
        block.transactions.len()
      )));
    }

    let cb = crate::index::block::util::extract_cbtx_fields(block.transactions.first(), 0);
    let header = HyDbHeader {
      hash: Self::block_hash(raw)?,
      prev_hash: HyDbHash(block.header.prev_hash.to_bytes()),
      merkle_root: HyDbHash(block.header.merkle_root.to_bytes()),
      time: block.header.time,
      bits: block.header.bits,
      difficulty: crate::consensus::bits_to_difficulty(block.header.bits),
      nonce: block.header.nonce,
      version: block.header.version,
      size: raw.len() as u32,
      tx_count: block.transactions.len() as u32,
      // Cumulative chainwork is not derivable from a single block in
      // isolation; the storage pipeline accumulates it from the parent row's
      // stored chainwork plus this block's `bits` at connect time.
      chainwork: None,
      coinbase: cb.height.is_some().then(|| HyDbCbFields {
        cb_height: cb.height,
        cb_merkle_root: cb.merkle_root.clone(),
        cb_merkle_flags: cb.merkle_flags,
        cb_best_cl_height: cb.best_cl_height,
        cb_best_cl_sig: cb.best_cl_sig.clone(),
        cb_credit_pool: cb.credit_pool,
      }),
    };

    let mut txs = Vec::with_capacity(block.transactions.len());
    for (pos, transaction) in block.transactions.iter().enumerate() {
      let (start, end) = ranges[pos];
      let txid = dash_primitives::hash::tx_hash(&raw[start..end]).to_bytes();
      txs.push(self.decode_tx(transaction, HyDbHash(txid), pos, (end - start) as u32)?);
    }

    Ok(HyDbBlock { header, txs })
  }
}

impl HyDbBlockSource {
  fn decode_tx(
    &self,
    transaction: &Transaction,
    txid: HyDbHash,
    block_pos: usize,
    size: u32,
  ) -> HyDbResult<HyDbTx> {
    let inputs = transaction
      .inputs
      .iter()
      .enumerate()
      .map(|(vin_index, input)| {
        let coinbase = input.prevout.hash.is_null();
        HyDbTxIn {
          vin_index: vin_index as u32,
          prev_outpoint: (!coinbase).then(|| HyDbOutpoint {
            txid: HyDbHash(input.prevout.hash.to_bytes()),
            vout: input.prevout.index,
          }),
          coinbase,
          script_sig: input.script_sig.as_bytes().to_vec(),
          sequence: input.sequence,
        }
      })
      .collect::<Vec<_>>();

    let outputs = transaction
      .outputs
      .iter()
      .enumerate()
      .map(|(vout, output)| {
        let script = output.script_pubkey.as_bytes();
        Ok(HyDbTxOut {
          vout: vout as u32,
          // Checked, not cast: a malformed transaction claiming more than
          // `i64::MAX` duffs would otherwise arrive as a negative value and be
          // summed into the supply and every address balance that touches it.
          value: HyDbAmount::from_sat(output.value.to_sat())?,
          script_pubkey: script.to_vec(),
          address: dash_script::derive_address(script, self.p2pkh_version, self.p2sh_version)
            .map(HyDbAddress),
          address_type: addr_type(script),
        })
      })
      .collect::<HyDbResult<Vec<_>>>()?;

    let value_out = HyDbAmount::try_sum(outputs.iter().map(|o| o.value))?.0;
    let (payload, validator_events) = self.decode_special(transaction, txid.0);

    Ok(HyDbTx {
      txid,
      block_pos: block_pos as u32,
      version: i32::from(transaction.version),
      tx_type: tx_type(transaction.tx_type),
      lock_time: transaction.lock_time,
      size,
      value_out: HyDbAmount(value_out),
      inputs,
      outputs,
      payload,
      validator_events,
    })
  }

  /// Decode a transaction's special-tx payload plus any validator lifecycle
  /// events. Non-special transactions return `(None, [])`.
  fn decode_special(
    &self,
    transaction: &Transaction,
    txid: [u8; 32],
  ) -> (Option<HyDbPayload>, Vec<HyDbVlEvent>) {
    if matches!(transaction.tx_type, TxType::Spend | TxType::Unknown(_)) {
      return (None, Vec::new());
    }
    let payload = match SpecialPayload::decode(transaction.tx_type, &transaction.extra_payload) {
      Ok(payload) => payload,
      Err(e) => {
        // Dropping the payload is data loss; name the transaction so the
        // gap is diagnosable rather than silent.
        tracing::warn!(
          txid = %hex_display(&txid),
          tx_type = ?transaction.tx_type,
          error = %e,
          "special-tx payload failed to decode; storing transaction without it"
        );
        return (None, Vec::new());
      }
    };

    let (protx_hash, json) = crate::tx::payload_for_storage(&payload, &txid);
    let hy_payload = Some(HyDbPayload {
      protx_hash: protx_hash.and_then(|h| HyDbHash::from_slice(&h).ok()),
      json,
    });

    let events =
      match crate::tx::validator_effect(&payload, &txid, self.p2pkh_version, self.p2sh_version) {
        Some(effect) => {
          let deliberate_drop = matches!(effect, ValidatorEffect::ReapSpentCollateral { .. });
          match validator_event(effect) {
            Some(event) => vec![event],
            None if deliberate_drop => Vec::new(),
            None => {
              tracing::warn!(
                txid = %hex_display(&txid),
                "validator event could not be mapped for storage; dropping it"
              );
              Vec::new()
            }
          }
        }
        None => Vec::new(),
      };
    (hy_payload, events)
  }
}

/// Render a txid in display order for logs.
fn hex_display(txid: &[u8; 32]) -> String {
  txid.iter().rev().map(|b| format!("{b:02x}")).collect()
}

/// Map a Dash `TxType` onto the storage-layer enum (Normal/Unknown → Normal).
fn tx_type(tx_type: TxType) -> HyDbTxType {
  match tx_type {
    TxType::Spend | TxType::Unknown(_) => HyDbTxType::Normal,
    TxType::ProviderRegister => HyDbTxType::ProviderRegister,
    TxType::ProviderUpdateService => HyDbTxType::ProviderUpdateService,
    TxType::ProviderUpdateRegistrar => HyDbTxType::ProviderUpdateRegistrar,
    TxType::ProviderUpdateRevoke => HyDbTxType::ProviderUpdateRevoke,
    TxType::CoinbaseCommitment => HyDbTxType::Coinbase,
    TxType::QuorumCommitment => HyDbTxType::QuorumCommitment,
    TxType::AssetLock => HyDbTxType::AssetLock,
    TxType::AssetUnlock => HyDbTxType::AssetUnlock,
    TxType::MnhfSignal => HyDbTxType::Mnhf,
  }
}

/// Classify an output script into the storage-layer address kind.
fn addr_type(script: &[u8]) -> HyDbAddrType {
  use dash_script::{classify as classify_script, ScriptKind};
  match classify_script(script) {
    ScriptKind::P2pkh => HyDbAddrType::P2pkh,
    ScriptKind::P2sh => HyDbAddrType::P2sh,
    ScriptKind::P2pk => HyDbAddrType::P2pk,
    _ => HyDbAddrType::Unknown,
  }
}

/// Normalize a daemon validator-type string onto the storage enum.
fn vl_type(value: &str) -> HyDbVlType {
  let compact = value
    .chars()
    .filter(|ch| ch.is_ascii_alphanumeric())
    .flat_map(char::to_lowercase)
    .collect::<String>();
  match compact.as_str() {
    "regular" => HyDbVlType::Regular,
    "highperformance" | "evo" | "evonode" => HyDbVlType::Evonode,
    _ => HyDbVlType::Unknown,
  }
}

/// Map a daemon [`ValidatorEffect`] onto the storage-layer delta event.
/// `ReapSpentCollateral` is intentionally dropped — the crate reaps validators
/// itself from spend resolution.
fn validator_event(effect: ValidatorEffect) -> Option<HyDbVlEvent> {
  match effect {
    ValidatorEffect::Insert(insert) => Some(HyDbVlEvent::Register(register(insert)?)),
    ValidatorEffect::UpdateService {
      pro_tx_hash,
      core_addrs,
      plat_p2p_addrs,
      plat_https_addrs,
      operator_payout_addr,
    } => Some(HyDbVlEvent::UpdateService(HyDbVlUpdService {
      protx_hash: HyDbHash::from_slice(&pro_tx_hash).ok()?,
      addr_core_p2p: core_addrs,
      addr_platform_p2p: plat_p2p_addrs,
      addr_platform_https: plat_https_addrs,
      operator_payout_address: operator_payout_addr.map(HyDbAddress),
    })),
    ValidatorEffect::UpdateRegistrar {
      pro_tx_hash,
      operator_pubkey,
      payout_addr,
      voting_addr,
    } => Some(HyDbVlEvent::UpdateRegistrar(HyDbVlUpdRegistrar {
      protx_hash: HyDbHash::from_slice(&pro_tx_hash).ok()?,
      operator_pubkey,
      payout_address: payout_addr.map(HyDbAddress),
      voting_address: voting_addr.map(HyDbAddress),
    })),
    ValidatorEffect::Revoke { pro_tx_hash } => Some(HyDbVlEvent::Revoke(HyDbVlRevoke {
      protx_hash: HyDbHash::from_slice(&pro_tx_hash).ok()?,
    })),
    ValidatorEffect::ReapSpentCollateral { .. } => None,
  }
}

/// Build the full registry row a ProRegTx creates.
fn register(insert: ValidatorInsert) -> Option<HyDbVlRegister> {
  Some(HyDbVlRegister {
    protx_hash: HyDbHash::from_slice(&insert.pro_tx_hash).ok()?,
    collateral_hash: HyDbHash::from_slice(&insert.collateral_hash).ok()?,
    collateral_index: insert.collateral_index,
    collateral_address: insert.collateral_address.map(HyDbAddress),
    owner_address: insert.owner_address.map(HyDbAddress),
    operator_pubkey: Some(insert.operator_pubkey),
    voting_address: insert.voting_address.map(HyDbAddress),
    addr_core_p2p: insert.core_addrs,
    addr_platform_p2p: insert.plat_p2p_addrs,
    addr_platform_https: insert.plat_https_addrs,
    payout_address: insert.payout_addr.map(HyDbAddress),
    operator_payout_address: insert.operator_payout_addr.map(HyDbAddress),
    operator_reward: insert.operator_reward,
    validator_type: vl_type(&insert.validator_type),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use hyphen_db::HyDbDataSrc;

  /// Mainnet block 1, whole. Small enough to embed (186 bytes) and real, which
  /// matters here: this seam has to reproduce identical facts from identical
  /// bytes for reorg re-derivation to be sound, so the fixture is a block the
  /// chain actually contains rather than a hand-built one.
  const BLOCK_1: &str = concat!(
    "02000000b67a40f3cd5804437a108f105533739c37e6229bc1adcab385140b59fd0f0000",
    "a71c1aade44bf8425bec0deb611c20b16da3442818ef20489ca1e2512be43eef814cdb52",
    "f0ff0f1edbf7010001010000000100000000000000000000000000000000000000000000",
    "00000000000000000000ffffffff0a510101062f503253482fffffffff0100743ba40b00",
    "0000232103a69850243c993c0645a6e8b38c774174174cc766cd3ec2140afd24d831b84c",
    "41ac00000000"
  );

  fn block_1() -> Vec<u8> {
    hex::decode(BLOCK_1).expect("fixture is valid hex")
  }

  fn src() -> HyDbBlockSource {
    // Mainnet address versions.
    HyDbBlockSource::new(0x4c, 0x10)
  }

  /// The header-only decode must agree with the whole-block decode on every
  /// header field. `identify` reads the header alone now, and what keeps that
  /// safe is that a block's header *is* its first 80 bytes — asserted here
  /// rather than trusted.
  #[test]
  fn the_header_only_decode_agrees_with_the_whole_block_decode() {
    let raw = block_1();
    let header = HyDbBlockSource::decode_header(&raw).expect("header decodes");
    let block = HyDbBlockSource::decode_block(&raw).expect("block decodes");

    assert_eq!(
      header.prev_hash.to_bytes(),
      block.header.prev_hash.to_bytes()
    );
    assert_eq!(
      header.merkle_root.to_bytes(),
      block.header.merkle_root.to_bytes()
    );
    assert_eq!(header.time, block.header.time);
    assert_eq!(header.bits, block.header.bits);
    assert_eq!(header.nonce, block.header.nonce);
    assert_eq!(header.version, block.header.version);
  }

  /// `identify` must name this block's parent as genesis. Asserted against the
  /// real hash rather than against another decode of the same bytes, so a
  /// decoder that is self-consistently wrong still fails.
  #[test]
  fn identify_names_the_genesis_parent() {
    let id = src().identify(&block_1()).expect("identify succeeds");
    let mut prev = id.prev_hash.0;
    prev.reverse();
    assert_eq!(
      hex::encode(prev),
      "00000ffd590b1485b3caadc19b22e6379c733355108f107a430458cdf3407ab6"
    );
  }

  /// The facts `decode` derives, against what the chain says they are.
  #[test]
  fn decode_reproduces_the_blocks_real_facts() {
    let raw = block_1();
    let decoded = src().decode(&raw).expect("decode succeeds");

    assert_eq!(decoded.header.size, 186);
    assert_eq!(decoded.header.tx_count, 1);
    assert_eq!(decoded.txs.len(), 1);

    let tx = &decoded.txs[0];
    let mut txid = tx.txid.0;
    txid.reverse();
    assert_eq!(
      hex::encode(txid),
      "ef3ee42b51e2a19c4820ef182844a36db1201c61eb0dec5b42f84be4ad1a1ca7"
    );
    // A block-1 coinbase: one input, marked as such, and one output.
    assert_eq!(tx.inputs.len(), 1);
    assert!(tx.inputs[0].coinbase);
    assert_eq!(tx.outputs.len(), 1);
  }
}
