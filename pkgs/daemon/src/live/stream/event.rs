//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Typed public live events exposed through the streamable HTTP API.

use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use crate::sources::ZmqEvent;
use crate::utils::{self, HashEndian};

/// Stream payload for one mempool transaction event.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MempoolTxEvent {
  pub sequence: u32,
  pub txid: String,
  pub rawtx: String,
}

/// Stream payload for one ChainLock update.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChainLockEvent {
  pub sequence: Option<u32>,
  pub block_hash: String,
  pub block_height: Option<i32>,
  pub raw_chainlock: String,
}

/// Stream payload for one governance object refresh or delta.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceObjectEvent {
  pub sequence: Option<u32>,
  pub object_hash: Option<String>,
  pub payload: Value,
}

/// Stream payload for one governance vote refresh or delta.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceVoteEvent {
  pub sequence: Option<u32>,
  pub object_hash: Option<String>,
  pub vote_hash: Option<String>,
  pub payload: Value,
}

/// Stream payload for one network snapshot update.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEvent {
  pub payload: Value,
}

/// One streamable live event emitted by the daemon.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
  MempoolTx {
    ts: u64,
    payload: MempoolTxEvent,
  },
  ChainLock {
    ts: u64,
    payload: ChainLockEvent,
  },
  GovernanceObject {
    ts: u64,
    payload: GovernanceObjectEvent,
  },
  GovernanceVote {
    ts: u64,
    payload: GovernanceVoteEvent,
  },
  Network {
    ts: u64,
    payload: NetworkEvent,
  },
  Heartbeat {
    ts: u64,
  },
}

impl LiveEvent {
  /// Construct one mempool transaction event from a raw transaction payload.
  pub fn mempool_tx(raw_tx: Vec<u8>, sequence: u32) -> Self {
    Self::MempoolTx {
      ts: now_epoch_secs(),
      payload: MempoolTxEvent {
        sequence,
        txid: utils::view_hex(
          &dash_primitives::hash::tx_hash(&raw_tx).to_bytes(),
          HashEndian::Ser,
        ),
        rawtx: hex::encode(raw_tx),
      },
    }
  }

  /// Convert a low-level ZMQ event into a public stream event when applicable.
  pub fn from_zmq(event: ZmqEvent) -> Option<Self> {
    let ts = now_epoch_secs();
    match event {
      ZmqEvent::RawTx { .. } => None,
      ZmqEvent::RawTxLock { .. } => None,
      ZmqEvent::RawChainLock { .. } => None,
      ZmqEvent::RawGovernanceObject { object, sequence } => Some(Self::GovernanceObject {
        ts,
        payload: GovernanceObjectEvent {
          sequence: Some(sequence),
          object_hash: None,
          payload: Value::String(hex::encode(object)),
        },
      }),
      ZmqEvent::RawGovernanceVote { vote, sequence } => Some(Self::GovernanceVote {
        ts,
        payload: GovernanceVoteEvent {
          sequence: Some(sequence),
          object_hash: None,
          vote_hash: None,
          payload: Value::String(hex::encode(vote)),
        },
      }),
      ZmqEvent::Sequence { .. } => None,
    }
  }

  /// Construct one network snapshot update event.
  pub fn network(payload: Value) -> Self {
    Self::Network {
      ts: now_epoch_secs(),
      payload: NetworkEvent { payload },
    }
  }

  /// Construct one governance object refresh event.
  pub fn governance_object(
    object_hash: Option<String>,
    sequence: Option<u32>,
    payload: Value,
  ) -> Self {
    Self::GovernanceObject {
      ts: now_epoch_secs(),
      payload: GovernanceObjectEvent {
        sequence,
        object_hash,
        payload,
      },
    }
  }

  /// Construct one governance vote refresh event.
  pub fn governance_vote(
    object_hash: Option<String>,
    vote_hash: Option<String>,
    sequence: Option<u32>,
    payload: Value,
  ) -> Self {
    Self::GovernanceVote {
      ts: now_epoch_secs(),
      payload: GovernanceVoteEvent {
        sequence,
        object_hash,
        vote_hash,
        payload,
      },
    }
  }

  /// Construct one ChainLock snapshot event.
  pub fn chainlock(block_hash: String, block_height: Option<i32>, raw_chainlock: String) -> Self {
    Self::ChainLock {
      ts: now_epoch_secs(),
      payload: ChainLockEvent {
        sequence: None,
        block_hash,
        block_height,
        raw_chainlock,
      },
    }
  }

  /// Construct a heartbeat event for idle periods.
  pub fn heartbeat() -> Self {
    Self::Heartbeat {
      ts: now_epoch_secs(),
    }
  }
}

fn now_epoch_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::LiveEvent;
  use crate::sources::ZmqEvent;

  #[test]
  fn raw_tx_zmq_event_becomes_mempool_stream_event() {
    let event = LiveEvent::mempool_tx(vec![1, 2, 3], 7);
    let json = serde_json::to_value(event).expect("stream event should serialize");
    assert_eq!(json["type"], "mempool_tx");
    assert_eq!(json["payload"]["sequence"], 7);
    assert_eq!(json["payload"]["rawtx"], hex::encode([1u8, 2, 3]));
  }

  #[test]
  fn network_event_uses_typed_payload_envelope() {
    let event = LiveEvent::network(json!({ "peerCount": 8 }));
    let json = serde_json::to_value(event).expect("network event should serialize");
    assert_eq!(json["type"], "network");
    assert_eq!(json["payload"]["payload"]["peerCount"], 8);
  }

  #[test]
  fn chainlock_snapshot_event_uses_public_payload_shape() {
    let event = LiveEvent::chainlock("ab".repeat(32), Some(42), "cd".repeat(48));
    let json = serde_json::to_value(event).expect("chainlock event should serialize");
    assert_eq!(json["type"], "chain_lock");
    assert_eq!(
      json["payload"]["blockHash"],
      serde_json::json!("ab".repeat(32))
    );
    assert_eq!(json["payload"]["blockHeight"], 42);
    assert_eq!(
      json["payload"]["rawChainlock"],
      serde_json::json!("cd".repeat(48))
    );
    assert!(json["payload"]["sequence"].is_null());
  }

  #[test]
  fn raw_chainlock_zmq_event_stays_internal_until_typed_snapshot_exists() {
    assert!(LiveEvent::from_zmq(ZmqEvent::RawChainLock {
      chainlock: vec![0xaa],
      sequence: 9,
    })
    .is_none());
  }

  #[test]
  fn governance_snapshot_events_keep_hashes_and_payloads() {
    let object =
      LiveEvent::governance_object(Some("11".repeat(32)), None, json!({ "name": "proposal" }));
    let vote = LiveEvent::governance_vote(
      Some("11".repeat(32)),
      Some("22".repeat(32)),
      None,
      json!({ "decision": "yes" }),
    );

    let object_json = serde_json::to_value(object).expect("governance object should serialize");
    assert_eq!(object_json["type"], "governance_object");
    assert_eq!(
      object_json["payload"]["objectHash"],
      serde_json::json!("11".repeat(32))
    );
    assert_eq!(object_json["payload"]["payload"]["name"], "proposal");

    let vote_json = serde_json::to_value(vote).expect("governance vote should serialize");
    assert_eq!(vote_json["type"], "governance_vote");
    assert_eq!(
      vote_json["payload"]["objectHash"],
      serde_json::json!("11".repeat(32))
    );
    assert_eq!(
      vote_json["payload"]["voteHash"],
      serde_json::json!("22".repeat(32))
    );
    assert_eq!(vote_json["payload"]["payload"]["decision"], "yes");
  }

  #[test]
  fn governance_zmq_event_uses_hex_payload_fallback() {
    let event = LiveEvent::from_zmq(ZmqEvent::RawGovernanceVote {
      vote: vec![0xde, 0xad, 0xbe, 0xef],
      sequence: 9,
    })
    .expect("raw governance vote should stream");
    let json = serde_json::to_value(event).expect("governance vote should serialize");
    assert_eq!(json["type"], "governance_vote");
    assert_eq!(json["payload"]["sequence"], 9);
    assert_eq!(json["payload"]["payload"], serde_json::json!("deadbeef"));
  }

  #[test]
  fn instantlock_events_stay_internal_only() {
    assert!(LiveEvent::from_zmq(ZmqEvent::RawTxLock {
      tx: vec![0xaa],
      sequence: 3,
    })
    .is_none());
  }
}
