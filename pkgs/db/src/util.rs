//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Small shared helpers: credential redaction for logs, `LIKE`-pattern
//! escaping for prefix search, and the 256-bit chainwork accumulation.

/// Redact the password from a connection URL for logging/serialization.
///
/// Splits at the *last* `@` before anything else about the string is trusted.
/// Locating the authority's end first is what leaked: a password holding an
/// unencoded `/` or `?` ended the authority ahead of its own `@`, so no `@`
/// remained to find and the whole URL was returned verbatim.
///
/// Percent-encoded passwords need no special handling. The span from `:` to
/// `@` is replaced whether or not its bytes are encoded, and decoding first
/// would only reintroduce the reserved characters this splits around.
///
/// A URL carrying no userinfo but a later `@`, such as one with an `@` in a
/// query value, is over-redacted. For a logging helper that is the correct
/// direction to fail in.
pub(crate) fn redact_url(url: &str) -> String {
  let (scheme, rest) = match url.find("://") {
    Some(idx) => (&url[..idx + 3], &url[idx + 3..]),
    None => ("", url),
  };
  let Some(at) = rest.rfind('@') else {
    return url.to_owned();
  };
  let userinfo = &rest[..at];
  match userinfo.find(':') {
    Some(colon) => format!("{scheme}{}:***{}", &userinfo[..colon], &rest[at..]),
    None => url.to_owned(),
  }
}

/// Escape `\`, `%`, and `_` in a user-supplied prefix so it is matched
/// literally by `LIKE`. Every query using this helper declares `ESCAPE '\'`
/// explicitly. Without this, a bare `%` prefix matches every row.
pub(crate) fn escape_like(prefix: &str) -> String {
  let mut out = String::with_capacity(prefix.len());
  for c in prefix.chars() {
    if matches!(c, '\\' | '%' | '_') {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

/// Normalize a display-order hex prefix for search: lowercase (hex is
/// case-insensitive to users) and `LIKE`-escaped. Applied uniformly on every
/// hash-search path — blocks, transactions, validators.
pub(crate) fn hex_prefix_pattern(prefix: &str) -> String {
  format!("{}%", escape_like(&prefix.to_ascii_lowercase()))
}

/// A minimal unsigned 256-bit integer for chainwork accumulation. Big-endian
/// limb order (`limbs[0]` is the most significant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct U256([u64; 4]);

impl U256 {
  pub(crate) const ZERO: Self = Self([0; 4]);
  pub(crate) const MAX: Self = Self([u64::MAX; 4]);

  pub(crate) fn is_zero(&self) -> bool {
    self.0 == [0; 4]
  }

  /// Wrapping addition (chainwork sums never overflow 256 bits in practice;
  /// wrapping keeps the arithmetic total).
  pub(crate) fn wrapping_add(self, rhs: Self) -> Self {
    let mut out = [0u64; 4];
    let mut carry = 0u64;
    for i in (0..4).rev() {
      let (sum, c1) = self.0[i].overflowing_add(rhs.0[i]);
      let (sum, c2) = sum.overflowing_add(carry);
      out[i] = sum;
      carry = u64::from(c1) + u64::from(c2);
    }
    Self(out)
  }

  fn not(self) -> Self {
    Self([!self.0[0], !self.0[1], !self.0[2], !self.0[3]])
  }

  fn shl1(self) -> Self {
    let mut out = [0u64; 4];
    for (i, limb) in out.iter_mut().enumerate() {
      *limb = self.0[i] << 1;
      if i + 1 < 4 {
        *limb |= self.0[i + 1] >> 63;
      }
    }
    Self(out)
  }

  fn bit(&self, idx: usize) -> bool {
    // idx 0 = most significant bit.
    (self.0[idx / 64] >> (63 - (idx % 64))) & 1 == 1
  }

  fn set_low_bit(mut self) -> Self {
    self.0[3] |= 1;
    self
  }

  fn ge(&self, rhs: &Self) -> bool {
    for i in 0..4 {
      if self.0[i] != rhs.0[i] {
        return self.0[i] > rhs.0[i];
      }
    }
    true
  }

  fn sub(self, rhs: Self) -> Self {
    let mut out = [0u64; 4];
    let mut borrow = 0u64;
    for i in (0..4).rev() {
      let (d, b1) = self.0[i].overflowing_sub(rhs.0[i]);
      let (d, b2) = d.overflowing_sub(borrow);
      out[i] = d;
      borrow = u64::from(b1) + u64::from(b2);
    }
    Self(out)
  }

  /// Binary long division, `self / rhs`. `rhs` must be non-zero.
  fn div(self, rhs: Self) -> Self {
    let mut quotient = Self::ZERO;
    let mut remainder = Self::ZERO;
    for i in 0..256 {
      remainder = remainder.shl1();
      if self.bit(i) {
        remainder = remainder.set_low_bit();
      }
      quotient = quotient.shl1();
      if remainder.ge(&rhs) {
        remainder = remainder.sub(rhs);
        quotient = quotient.set_low_bit();
      }
    }
    quotient
  }

  /// Parse from big-endian hex (up to 64 chars). Empty/invalid digits error.
  pub(crate) fn from_hex(hex_str: &str) -> Option<Self> {
    let s = hex_str.trim_start_matches("0x");
    if s.is_empty() || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
      return None;
    }
    let padded = format!("{s:0>64}");
    let mut limbs = [0u64; 4];
    for (i, limb) in limbs.iter_mut().enumerate() {
      *limb = u64::from_str_radix(&padded[i * 16..(i + 1) * 16], 16).ok()?;
    }
    Some(Self(limbs))
  }

  /// Render as fixed-width 64-char big-endian hex.
  pub(crate) fn to_hex(self) -> String {
    format!(
      "{:016x}{:016x}{:016x}{:016x}",
      self.0[0], self.0[1], self.0[2], self.0[3]
    )
  }
}

/// Expand a compact difficulty target (`nBits`) to a 256-bit target.
/// Returns `None` for a zero/overflowing/negative-mantissa encoding.
fn expand_compact(bits: u32) -> Option<U256> {
  let exponent = (bits >> 24) as usize;
  let mantissa = bits & 0x007f_ffff;
  if bits & 0x0080_0000 != 0 || mantissa == 0 {
    return None; // negative or zero target
  }
  if exponent > 32 {
    return None; // overflow
  }
  // target = mantissa * 256^(exponent - 3)
  let mut bytes = [0u8; 32];
  let m = mantissa.to_be_bytes(); // [0, b1, b2, b3]
  if exponent >= 3 {
    let top = 32usize.checked_sub(exponent)?;
    for (offset, byte) in m[1..].iter().enumerate() {
      let idx = top + offset;
      if idx >= 32 {
        break;
      }
      bytes[idx] = *byte;
    }
  } else {
    // Tiny exponent: shift the mantissa right instead.
    let shifted = mantissa >> (8 * (3 - exponent));
    bytes[28..].copy_from_slice(&shifted.to_be_bytes());
  }
  let mut limbs = [0u64; 4];
  for (i, limb) in limbs.iter_mut().enumerate() {
    let mut v = 0u64;
    for b in &bytes[i * 8..(i + 1) * 8] {
      v = (v << 8) | u64::from(*b);
    }
    *limb = v;
  }
  Some(U256(limbs))
}

/// The proof-of-work a block at difficulty `bits` contributes:
/// `floor(2^256 / (target + 1))`, computed as `(~target / (target + 1)) + 1`
/// (Bitcoin Core's `GetBlockProof` identity, which stays within 256 bits).
pub(crate) fn block_proof(bits: u32) -> U256 {
  let Some(target) = expand_compact(bits) else {
    return U256::ZERO;
  };
  if target.is_zero() || target == U256::MAX {
    return U256::ZERO;
  }
  let divisor = target.wrapping_add(U256::ZERO.set_low_bit());
  target
    .not()
    .div(divisor)
    .wrapping_add(U256::ZERO.set_low_bit())
}

/// Accumulate chainwork: the parent's cumulative work (64-char hex, `None` at
/// genesis) plus this block's proof from `bits`. Returns fixed-width hex.
pub(crate) fn accumulate_chainwork(prev: Option<&str>, bits: u32) -> Option<String> {
  let prev_work = match prev {
    Some(hex_str) => U256::from_hex(hex_str)?,
    None => U256::ZERO,
  };
  Some(prev_work.wrapping_add(block_proof(bits)).to_hex())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn redacts_password_at_last_at_sign() {
    let out = redact_url("postgresql://user:p@ss@host:5432/db");
    assert_eq!(out, "postgresql://user:***@host:5432/db");
  }

  #[test]
  fn redacts_schemeless_url() {
    let out = redact_url("user:secret@host/db");
    assert_eq!(out, "user:***@host/db");
    assert!(!out.contains("secret"));
  }

  #[test]
  fn leaves_urls_without_credentials_alone() {
    assert_eq!(redact_url("postgresql://host/db"), "postgresql://host/db");
    assert_eq!(
      redact_url("postgresql://user@host/db"),
      "postgresql://user@host/db"
    );
  }

  #[test]
  fn redacts_passwords_holding_reserved_characters() {
    // Each of these ended the authority before its own `@` under the old
    // split, so the function returned the URL — password included — and the
    // pool-creation log line printed it on every startup.
    for (url, want, secret) in [
      (
        "postgresql://alice:pa/ss@host/db",
        "postgresql://alice:***@host/db",
        "pa/ss",
      ),
      (
        "postgresql://alice:pa?ss@host/db",
        "postgresql://alice:***@host/db",
        "pa?ss",
      ),
      (
        "postgresql://alice:p/a?s/s@host:5432/db?sslmode=require",
        "postgresql://alice:***@host:5432/db?sslmode=require",
        "p/a?s/s",
      ),
    ] {
      let out = redact_url(url);
      assert_eq!(out, want, "redacting {url}");
      assert!(!out.contains(secret), "the password survived in {out}");
    }
  }

  #[test]
  fn redacts_a_socket_url_whose_password_holds_a_slash() {
    // The libpq `?host=` form the socket path arrives in, which is also the
    // shape whose parse failure reports the URL back to the operator.
    let out = redact_url("postgresql://alice:pa/ss@/db?host=/var/run/postgresql");
    assert_eq!(out, "postgresql://alice:***@/db?host=/var/run/postgresql");
    assert!(!out.contains("pa/ss"));
  }

  #[test]
  fn escapes_like_metacharacters() {
    assert_eq!(escape_like("a%b_c\\d"), "a\\%b\\_c\\\\d");
    assert_eq!(hex_prefix_pattern("AB%"), "ab\\%%");
  }

  #[test]
  fn block_proof_matches_known_value() {
    // Difficulty-1 target (0x1d00ffff): proof = 0x0100010001 (4295032833).
    let proof = block_proof(0x1d00_ffff);
    assert_eq!(
      proof.to_hex(),
      "0000000000000000000000000000000000000000000000000000000100010001"
    );
  }

  #[test]
  fn chainwork_accumulates() {
    let genesis = accumulate_chainwork(None, 0x1d00_ffff);
    assert_eq!(
      genesis.as_deref(),
      Some("0000000000000000000000000000000000000000000000000000000100010001")
    );
    let next = accumulate_chainwork(genesis.as_deref(), 0x1d00_ffff);
    assert_eq!(
      next.as_deref(),
      Some("0000000000000000000000000000000000000000000000000000000200020002")
    );
  }

  #[test]
  fn invalid_bits_contribute_zero_work() {
    assert_eq!(block_proof(0), U256::ZERO);
    assert_eq!(block_proof(0x0080_0000), U256::ZERO); // negative flag
    assert_eq!(block_proof(0xff00_0001), U256::ZERO); // absurd exponent
  }

  #[test]
  fn u256_hex_round_trip() {
    let hex_in = "deadbeef00000000000000000000000000000000000000000000000000000001";
    assert_eq!(
      U256::from_hex(hex_in).map(U256::to_hex).as_deref(),
      Some(hex_in)
    );
    assert!(U256::from_hex("").is_none());
    assert!(U256::from_hex("zz").is_none());
  }
}
