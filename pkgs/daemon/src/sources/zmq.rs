//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! ZMQ source module for live node event subscriptions.

use std::convert::TryInto;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::info;
use zeromq::{Socket, SocketRecv, SubSocket, ZmqMessage};

use super::ZmqSource as ZmqSourceTrait;
use crate::config::{ZmqConfig, ZmqTopic};
use crate::error::{DaemonError, RestErrorKind};
use hyphend::wire::msg::ZmqDiag;

const ZMQ_STARTUP_TIMEOUT_SECS: u64 = 5;

/// Raw multipart message received from a node ZMQ subscription.
#[derive(Debug, Clone)]
pub struct RawZmqMessage {
  pub frames: Vec<Vec<u8>>,
}

/// Typed low-level ZMQ event decoded from a node multipart message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZmqEvent {
  RawTx { tx: Vec<u8>, sequence: u32 },
  RawTxLock { tx: Vec<u8>, sequence: u32 },
  RawChainLock { chainlock: Vec<u8>, sequence: u32 },
  RawGovernanceObject { object: Vec<u8>, sequence: u32 },
  RawGovernanceVote { vote: Vec<u8>, sequence: u32 },
  Sequence { event: SequenceEvent, sequence: u32 },
}

/// Typed interpretation of the node `sequence` ZMQ topic.
///
/// Hashes are in serialized byte order, the same order as `consensus::txid`,
/// block transaction hashes and RPC-derived keys. The node publishes them in
/// display order on the wire; `decode_sequence_event` reverses them once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceEvent {
  BlockConnected {
    hash: [u8; 32],
  },
  BlockDisconnected {
    hash: [u8; 32],
  },
  TxAdded {
    txid: [u8; 32],
    mempool_sequence: u64,
  },
  TxRemoved {
    txid: [u8; 32],
    mempool_sequence: u64,
  },
}

/// ZeroMQ subscriber transport for node live topics.
pub struct CoreZmqSource {
  endpoints: Vec<String>,
  topics: Vec<ZmqTopic>,
  socket: Mutex<SubSocket>,
  metrics: ZmqMetrics,
}

impl std::fmt::Debug for CoreZmqSource {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ZmqSource")
      .field("endpoints", &self.endpoints)
      .field("topics", &self.topics)
      .finish()
  }
}

#[derive(Debug)]
struct ZmqMetrics {
  enabled: bool,
  connected: AtomicBool,
  total_events: AtomicU64,
  total_errors: AtomicU64,
  last_event_epoch: AtomicU64,
  sequence_gaps: AtomicU64,
  topic_counts: StdMutex<Vec<(String, u64)>>,
  error_counts: StdMutex<Vec<(String, u64)>>,
  last_sequence: StdMutex<Vec<(String, u32)>>,
}

impl ZmqMetrics {
  fn new(config: &ZmqConfig) -> Self {
    Self {
      enabled: config.enabled,
      connected: AtomicBool::new(false),
      total_events: AtomicU64::new(0),
      total_errors: AtomicU64::new(0),
      last_event_epoch: AtomicU64::new(0),
      sequence_gaps: AtomicU64::new(0),
      topic_counts: StdMutex::new(Vec::new()),
      error_counts: StdMutex::new(Vec::new()),
      last_sequence: StdMutex::new(Vec::new()),
    }
  }

  fn mark_connected(&self) {
    self.connected.store(true, Ordering::Relaxed);
  }

  fn record_event(&self, topic: &str, sequence: u32) {
    self.total_events.fetch_add(1, Ordering::Relaxed);
    self.connected.store(true, Ordering::Relaxed);
    self.bump_counter(&self.topic_counts, topic);
    self.track_sequence(topic, sequence);
    let epoch = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    self.last_event_epoch.store(epoch, Ordering::Relaxed);
  }

  fn record_error(&self, category: &str) {
    self.total_errors.fetch_add(1, Ordering::Relaxed);
    self.connected.store(false, Ordering::Relaxed);
    self.bump_counter(&self.error_counts, category);
  }

  fn snapshot(&self, endpoints: &[String], topics: &[ZmqTopic]) -> ZmqDiag {
    let last_event_epoch = self.last_event_epoch.load(Ordering::Relaxed);
    ZmqDiag {
      enabled: self.enabled,
      connected: self.connected.load(Ordering::Relaxed),
      endpoints: endpoints.to_vec(),
      subscriptions: topics
        .iter()
        .map(|topic| topic.subscription_name().to_owned())
        .collect(),
      total_events: self.total_events.load(Ordering::Relaxed),
      total_errors: self.total_errors.load(Ordering::Relaxed),
      last_event_epoch: if last_event_epoch == 0 {
        None
      } else {
        Some(last_event_epoch)
      },
      sequence_gaps: self.sequence_gaps.load(Ordering::Relaxed),
      topic_counts: self
        .topic_counts
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

  fn bump_counter(&self, counters: &StdMutex<Vec<(String, u64)>>, key: &str) {
    if let Ok(mut counters) = counters.lock() {
      if let Some((_, count)) = counters.iter_mut().find(|(name, _)| name == key) {
        *count += 1;
      } else {
        counters.push((key.to_owned(), 1));
      }
    }
  }

  fn track_sequence(&self, topic: &str, sequence: u32) {
    if let Ok(mut last_sequence) = self.last_sequence.lock() {
      if let Some((_, last)) = last_sequence.iter_mut().find(|(name, _)| name == topic) {
        if *last != sequence.wrapping_sub(1) {
          self.sequence_gaps.fetch_add(1, Ordering::Relaxed);
        }
        *last = sequence;
      } else {
        last_sequence.push((topic.to_owned(), sequence));
      }
    }
  }
}

impl CoreZmqSource {
  /// Connect a subscriber socket to all configured ZMQ endpoints.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when topic subscription or endpoint
  /// connection fails.
  pub async fn connect(config: &ZmqConfig) -> Result<Self, DaemonError> {
    let mut socket = SubSocket::new();
    for topic in &config.subscribe {
      info!(
        topic = topic.subscription_name(),
        "subscribing to Core ZMQ topic"
      );
      timeout(
        Duration::from_secs(ZMQ_STARTUP_TIMEOUT_SECS),
        socket.subscribe(topic.subscription_name()),
      )
      .await
      .map_err(|_| DaemonError::RestSource {
        message: format!(
          "ZMQ subscribe timed out after {ZMQ_STARTUP_TIMEOUT_SECS}s: {}",
          topic.subscription_name()
        ),
        kind: RestErrorKind::Timeout,
      })?
      .map_err(|e| DaemonError::RestSource {
        message: format!("ZMQ subscribe failed: {e}"),
        kind: RestErrorKind::Connect,
      })?;
    }
    for endpoint in &config.connect {
      info!(%endpoint, "connecting to Core ZMQ endpoint");
      timeout(
        Duration::from_secs(ZMQ_STARTUP_TIMEOUT_SECS),
        socket.connect(endpoint),
      )
      .await
      .map_err(|_| DaemonError::RestSource {
        message: format!("ZMQ connect timed out after {ZMQ_STARTUP_TIMEOUT_SECS}s: {endpoint}"),
        kind: RestErrorKind::Timeout,
      })?
      .map_err(|e| DaemonError::RestSource {
        message: format!("ZMQ connect failed: {e}"),
        kind: RestErrorKind::Connect,
      })?;
    }
    let metrics = ZmqMetrics::new(config);
    metrics.mark_connected();
    Ok(Self {
      endpoints: config.connect.clone(),
      topics: config.subscribe.clone(),
      socket: Mutex::new(socket),
      metrics,
    })
  }

  /// Receive the next raw multipart message from the subscriber socket.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when socket receive fails.
  pub async fn recv_raw(&self) -> Result<RawZmqMessage, DaemonError> {
    let mut socket = self.socket.lock().await;
    let msg = socket.recv().await.map_err(|e| {
      self.metrics.record_error("receive");
      DaemonError::RestSource {
        message: format!("ZMQ receive failed: {e}"),
        kind: RestErrorKind::Other,
      }
    })?;
    Ok(RawZmqMessage {
      frames: split_message(msg),
    })
  }

  /// Receive and decode the next multipart ZMQ message.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when socket receive or message decoding
  /// fails.
  pub async fn recv_event(&self) -> Result<ZmqEvent, DaemonError> {
    let raw = self.recv_raw().await?;
    match decode_event(&raw) {
      Ok(event) => {
        self
          .metrics
          .record_event(event.topic_name(), event.sequence_number());
        Ok(event)
      }
      Err(err) => {
        self.metrics.record_error("decode");
        Err(err)
      }
    }
  }

  /// Return a typed ZMQ diagnostic snapshot.
  pub fn diag(&self) -> ZmqDiag {
    self.metrics.snapshot(&self.endpoints, &self.topics)
  }
}

#[async_trait::async_trait]
impl ZmqSourceTrait for CoreZmqSource {
  async fn recv_event(&self) -> Result<ZmqEvent, DaemonError> {
    CoreZmqSource::recv_event(self).await
  }

  fn diag(&self) -> ZmqDiag {
    CoreZmqSource::diag(self)
  }
}

impl ZmqTopic {
  fn subscription_name(self) -> &'static str {
    match self {
      Self::RawTx => "rawtx",
      Self::Sequence => "sequence",
      Self::RawTxLock => "rawtxlock",
      Self::RawChainLock => "rawchainlock",
      Self::RawGovernanceObject => "rawgovernanceobject",
      Self::RawGovernanceVote => "rawgovernancevote",
    }
  }
}

fn split_message(msg: ZmqMessage) -> Vec<Vec<u8>> {
  msg.iter().map(|frame| frame.to_vec()).collect()
}

fn decode_event(raw: &RawZmqMessage) -> Result<ZmqEvent, DaemonError> {
  if raw.frames.len() != 3 {
    return Err(DaemonError::RestSource {
      message: format!("ZMQ message must have 3 frames, got {}", raw.frames.len()),
      kind: RestErrorKind::Other,
    });
  }

  let topic = std::str::from_utf8(&raw.frames[0]).map_err(|e| DaemonError::RestSource {
    message: format!("ZMQ topic is not valid UTF-8: {e}"),
    kind: RestErrorKind::Other,
  })?;
  let payload = raw.frames[1].clone();
  let sequence = parse_u32_le(&raw.frames[2], "ZMQ sequence")?;

  match topic {
    "rawtx" => Ok(ZmqEvent::RawTx {
      tx: payload,
      sequence,
    }),
    "rawtxlock" => Ok(ZmqEvent::RawTxLock {
      tx: payload,
      sequence,
    }),
    "rawchainlock" => Ok(ZmqEvent::RawChainLock {
      chainlock: payload,
      sequence,
    }),
    "rawgovernanceobject" => Ok(ZmqEvent::RawGovernanceObject {
      object: payload,
      sequence,
    }),
    "rawgovernancevote" => Ok(ZmqEvent::RawGovernanceVote {
      vote: payload,
      sequence,
    }),
    "sequence" => Ok(ZmqEvent::Sequence {
      event: decode_sequence_event(&payload)?,
      sequence,
    }),
    _ => Err(DaemonError::RestSource {
      message: format!("unsupported ZMQ topic: {topic}"),
      kind: RestErrorKind::Other,
    }),
  }
}

impl ZmqEvent {
  fn topic_name(&self) -> &'static str {
    match self {
      Self::RawTx { .. } => "rawtx",
      Self::RawTxLock { .. } => "rawtxlock",
      Self::RawChainLock { .. } => "rawchainlock",
      Self::RawGovernanceObject { .. } => "rawgovernanceobject",
      Self::RawGovernanceVote { .. } => "rawgovernancevote",
      Self::Sequence { .. } => "sequence",
    }
  }

  fn sequence_number(&self) -> u32 {
    match self {
      Self::RawTx { sequence, .. }
      | Self::RawTxLock { sequence, .. }
      | Self::RawChainLock { sequence, .. }
      | Self::RawGovernanceObject { sequence, .. }
      | Self::RawGovernanceVote { sequence, .. }
      | Self::Sequence { sequence, .. } => *sequence,
    }
  }
}

fn decode_sequence_event(payload: &[u8]) -> Result<SequenceEvent, DaemonError> {
  if payload.len() < 33 {
    return Err(DaemonError::RestSource {
      message: format!("sequence payload too short: {}", payload.len()),
      kind: RestErrorKind::Other,
    });
  }

  let mut hash: [u8; 32] = payload[..32]
    .try_into()
    .map_err(|_| DaemonError::RestSource {
      message: "sequence hash must be 32 bytes".to_owned(),
      kind: RestErrorKind::Other,
    })?;
  // The node writes sequence hashes reversed (display order); every other key is serialized order.
  hash.reverse();
  let kind = payload[32];
  match kind {
    b'C' => {
      ensure_sequence_len(payload, 33)?;
      Ok(SequenceEvent::BlockConnected { hash })
    }
    b'D' => {
      ensure_sequence_len(payload, 33)?;
      Ok(SequenceEvent::BlockDisconnected { hash })
    }
    b'A' => {
      ensure_sequence_len(payload, 41)?;
      Ok(SequenceEvent::TxAdded {
        txid: hash,
        mempool_sequence: parse_u64_le(&payload[33..41], "sequence add mempool number")?,
      })
    }
    b'R' => {
      ensure_sequence_len(payload, 41)?;
      Ok(SequenceEvent::TxRemoved {
        txid: hash,
        mempool_sequence: parse_u64_le(&payload[33..41], "sequence remove mempool number")?,
      })
    }
    _ => Err(DaemonError::RestSource {
      message: format!("unsupported sequence action byte: {kind:#x}"),
      kind: RestErrorKind::Other,
    }),
  }
}

fn ensure_sequence_len(payload: &[u8], expected: usize) -> Result<(), DaemonError> {
  if payload.len() != expected {
    return Err(DaemonError::RestSource {
      message: format!(
        "sequence payload length mismatch: expected {expected}, got {}",
        payload.len()
      ),
      kind: RestErrorKind::Other,
    });
  }
  Ok(())
}

fn parse_u32_le(bytes: &[u8], field: &str) -> Result<u32, DaemonError> {
  let raw: [u8; 4] = bytes.try_into().map_err(|_| DaemonError::RestSource {
    message: format!("{field} must be 4 bytes"),
    kind: RestErrorKind::Other,
  })?;
  Ok(u32::from_le_bytes(raw))
}

fn parse_u64_le(bytes: &[u8], field: &str) -> Result<u64, DaemonError> {
  let raw: [u8; 8] = bytes.try_into().map_err(|_| DaemonError::RestSource {
    message: format!("{field} must be 8 bytes"),
    kind: RestErrorKind::Other,
  })?;
  Ok(u64::from_le_bytes(raw))
}

/// Wire-format fixtures for the `sequence` topic, shared by consumer tests.
#[cfg(test)]
pub(crate) mod test_wire {
  use super::{decode_event, RawZmqMessage, ZmqEvent};

  /// Display-order txid of [`raw_tx`], as RPC prints it and the node sends it on `sequence`.
  pub(crate) const RAW_TX_DISPLAY_TXID: &str =
    "ef3ee42b51e2a19c4820ef182844a36db1201c61eb0dec5b42f84be4ad1a1ca7";

  pub(crate) fn raw_tx() -> Vec<u8> {
    hex::decode(concat!(
      "01000000",
      "01",
      "0000000000000000000000000000000000000000000000000000000000000000",
      "ffffffff",
      "0a510101062f503253482f",
      "ffffffff",
      "01",
      "00743ba40b000000",
      "23",
      "2103a69850243c993c0645a6e8b38c774174174cc766cd3ec2140afd24d831b84c41ac",
      "00000000"
    ))
    .expect("valid transaction fixture")
  }

  /// Decode one `sequence` message laid out like the node's `SendSequenceMsg`:
  /// the hash reversed into display order, a label, then an optional mempool sequence.
  pub(crate) fn sequence(
    display_hash: &str,
    label: u8,
    mempool_sequence: Option<u64>,
    sequence: u32,
  ) -> ZmqEvent {
    let mut payload = hex::decode(display_hash).expect("display hash hex");
    payload.push(label);
    if let Some(mempool_sequence) = mempool_sequence {
      payload.extend_from_slice(&mempool_sequence.to_le_bytes());
    }
    decode_event(&RawZmqMessage {
      frames: vec![
        b"sequence".to_vec(),
        payload,
        sequence.to_le_bytes().to_vec(),
      ],
    })
    .expect("sequence message should decode")
  }
}

#[cfg(test)]
mod tests {
  use bytes::Bytes;

  use super::{
    decode_event, decode_sequence_event, split_message, RawZmqMessage, SequenceEvent, ZmqEvent,
    ZmqMetrics,
  };
  use crate::config::{ZmqConfig, ZmqTopic};

  #[test]
  fn zmq_topics_map_to_dash_subscription_names() {
    assert_eq!(ZmqTopic::RawTx.subscription_name(), "rawtx");
    assert_eq!(ZmqTopic::Sequence.subscription_name(), "sequence");
    assert_eq!(ZmqTopic::RawTxLock.subscription_name(), "rawtxlock");
    assert_eq!(ZmqTopic::RawChainLock.subscription_name(), "rawchainlock");
    assert_eq!(
      ZmqTopic::RawGovernanceObject.subscription_name(),
      "rawgovernanceobject"
    );
    assert_eq!(
      ZmqTopic::RawGovernanceVote.subscription_name(),
      "rawgovernancevote"
    );
  }

  #[test]
  fn split_message_preserves_frame_boundaries() {
    let msg = zeromq::ZmqMessage::try_from(vec![
      Bytes::from_static(b"rawtx"),
      Bytes::from_static(&[1, 2, 3]),
      Bytes::from_static(&[4, 5, 6, 7]),
    ])
    .expect("multipart message should build");
    assert_eq!(
      split_message(msg),
      vec![b"rawtx".to_vec(), vec![1, 2, 3], vec![4, 5, 6, 7]]
    );
  }

  #[test]
  fn decode_rawtx_event_reads_payload_and_sequence() {
    let raw = RawZmqMessage {
      frames: vec![
        b"rawtx".to_vec(),
        vec![0xaa, 0xbb],
        7u32.to_le_bytes().to_vec(),
      ],
    };
    assert_eq!(
      decode_event(&raw).expect("rawtx event should decode"),
      ZmqEvent::RawTx {
        tx: vec![0xaa, 0xbb],
        sequence: 7,
      }
    );
  }

  #[test]
  fn decode_sequence_added_event_reads_mempool_sequence() {
    let mut payload = vec![0x11; 32];
    payload.push(b'A');
    payload.extend_from_slice(&42u64.to_le_bytes());
    assert_eq!(
      decode_sequence_event(&payload).expect("sequence add should decode"),
      SequenceEvent::TxAdded {
        txid: [0x11; 32],
        mempool_sequence: 42,
      }
    );
  }

  #[test]
  fn sequence_txids_decode_to_the_serialized_order_of_the_raw_transaction() {
    let raw = super::test_wire::raw_tx();
    let txid = crate::consensus::txid(&raw).expect("transaction txid");
    assert_ne!(
      hex::encode(txid),
      super::test_wire::RAW_TX_DISPLAY_TXID,
      "fixture must be asymmetric"
    );
    for (label, expected) in [
      (
        b'A',
        SequenceEvent::TxAdded {
          txid,
          mempool_sequence: 42,
        },
      ),
      (
        b'R',
        SequenceEvent::TxRemoved {
          txid,
          mempool_sequence: 42,
        },
      ),
    ] {
      assert_eq!(
        super::test_wire::sequence(super::test_wire::RAW_TX_DISPLAY_TXID, label, Some(42), 3),
        ZmqEvent::Sequence {
          event: expected,
          sequence: 3,
        }
      );
    }
  }

  #[test]
  fn sequence_block_hashes_decode_to_serialized_order_and_format_back_to_display() {
    // Mainnet genesis block, as `getblockhash 0` prints it.
    let display = "00000ffd590b1485b3caadc19b22e6379c733355108f107a430458cdf3407ab6";
    let mut serialized: [u8; 32] = hex::decode(display)
      .expect("display hash")
      .try_into()
      .expect("32 bytes");
    serialized.reverse();
    for (label, expected) in [
      (b'C', SequenceEvent::BlockConnected { hash: serialized }),
      (b'D', SequenceEvent::BlockDisconnected { hash: serialized }),
    ] {
      let ZmqEvent::Sequence { event, .. } = super::test_wire::sequence(display, label, None, 1)
      else {
        panic!("sequence event expected");
      };
      assert_eq!(event, expected);
      let (SequenceEvent::BlockConnected { hash } | SequenceEvent::BlockDisconnected { hash }) =
        event
      else {
        panic!("block event expected");
      };
      assert_eq!(
        crate::utils::view_hex(&hash, crate::utils::HashEndian::Ser),
        display
      );
    }
  }

  #[test]
  fn decode_sequence_block_connected_event_has_no_extra_payload() {
    let mut payload = vec![0x22; 32];
    payload.push(b'C');
    assert_eq!(
      decode_sequence_event(&payload).expect("sequence block connect should decode"),
      SequenceEvent::BlockConnected { hash: [0x22; 32] }
    );
  }

  #[test]
  fn decode_event_rejects_unknown_topic() {
    let raw = RawZmqMessage {
      frames: vec![b"unknown".to_vec(), vec![1], 1u32.to_le_bytes().to_vec()],
    };
    let err = decode_event(&raw).expect_err("unknown topic should fail");
    assert!(matches!(
      err,
      crate::error::DaemonError::RestSource { message, .. }
      if message.contains("unsupported ZMQ topic")
    ));
  }

  #[test]
  fn zmq_metrics_track_topic_counts_and_sequence_gaps() {
    let metrics = ZmqMetrics::new(&ZmqConfig::default());
    metrics.record_event("rawtx", 1);
    metrics.record_event("rawtx", 2);
    metrics.record_event("rawtx", 4);
    metrics.record_error("decode");

    let diag = metrics.snapshot(&["tcp://127.0.0.1:28332".to_owned()], &[ZmqTopic::RawTx]);
    assert!(diag.connected == false);
    assert_eq!(diag.total_events, 3);
    assert_eq!(diag.total_errors, 1);
    assert_eq!(diag.sequence_gaps, 1);
    assert_eq!(diag.topic_counts, vec![("rawtx".to_owned(), 3)]);
    assert_eq!(diag.error_counts, vec![("decode".to_owned(), 1)]);
    assert_eq!(diag.subscriptions, vec!["rawtx".to_owned()]);
  }
}
