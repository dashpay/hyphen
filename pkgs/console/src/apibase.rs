//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Where the browse panel sends its REST requests.
//!
//! The console learns this from the daemon rather than assuming it. Every
//! heartbeat frame already carries `config.api`, so a console that hardcodes a
//! URL is ignoring the answer it is being handed — and breaks silently, with
//! connection errors on one panel while the rest of the UI works, whenever an
//! operator moves the bind or turns the API off.

/// What the console should do about REST, resolved from the flag and the
/// daemon's own reported configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiBase {
  /// Send browse requests here.
  Url(String),
  /// The daemon is serving no REST API, so the panel should say that rather
  /// than fail to connect to something that was never there.
  Disabled,
  /// No frame has arrived yet, so nothing is known either way.
  Unknown,
}

/// Turn a *bind* address into a URL that can actually be fetched.
///
/// A bind is not a host. Production binds `0.0.0.0:3080`, and
/// `http://0.0.0.0:3080` is not a thing a client can connect to, so the
/// wildcard forms have to become loopback — which is correct here because the
/// console only ever talks to a daemon it shares a datadir with. IPv6 literals
/// also have to keep their brackets, or the port parses as part of the address.
#[must_use]
pub fn url_from_bind(bind: &str) -> Option<String> {
  let bind = bind.trim();
  if bind.is_empty() {
    return None;
  }
  // Split host from port at the *last* colon, so an unbracketed IPv6 literal
  // does not get cut at its first group.
  let (host, port) = match bind.rsplit_once(':') {
    Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
      (host, port)
    }
    // No port at all: not a bind this console can turn into an endpoint.
    _ => return None,
  };
  let host = host.trim_matches(|c| c == '[' || c == ']');
  let host = match host {
    "0.0.0.0" | "" => "127.0.0.1",
    "::" | "::0" => "::1",
    other => other,
  };
  // Re-bracket if this is an IPv6 literal: a colon in the host is the tell,
  // since hostnames cannot contain one.
  if host.contains(':') {
    return Some(format!("http://[{host}]:{port}"));
  }
  Some(format!("http://{host}:{port}"))
}

/// Resolve where browse requests go.
///
/// An explicit `--api-url` wins outright, so an operator can point the console
/// at a daemon that is not the one whose socket it is reading — the one case
/// the daemon's own config cannot answer.
#[must_use]
pub fn resolve(override_url: Option<&str>, reported: Option<(bool, Option<&str>)>) -> ApiBase {
  if let Some(url) = override_url {
    return ApiBase::Url(url.trim_end_matches('/').to_owned());
  }
  match reported {
    None => ApiBase::Unknown,
    Some((false, _)) => ApiBase::Disabled,
    Some((true, bind)) => bind
      .and_then(url_from_bind)
      .map_or(ApiBase::Disabled, ApiBase::Url),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A wildcard bind is the production case and the one that cannot be used
  /// verbatim, so it is the case worth pinning.
  #[test]
  fn wildcard_binds_become_loopback() {
    assert_eq!(
      url_from_bind("0.0.0.0:3080").as_deref(),
      Some("http://127.0.0.1:3080")
    );
    assert_eq!(
      url_from_bind("[::]:3080").as_deref(),
      Some("http://[::1]:3080")
    );
  }

  #[test]
  fn explicit_hosts_are_kept() {
    assert_eq!(
      url_from_bind("127.0.0.1:3080").as_deref(),
      Some("http://127.0.0.1:3080")
    );
    assert_eq!(
      url_from_bind("10.1.2.3:8080").as_deref(),
      Some("http://10.1.2.3:8080")
    );
  }

  /// An unbracketed IPv6 literal must not be split at its first colon, and
  /// must come back bracketed or the port is ambiguous.
  #[test]
  fn ipv6_literals_keep_their_brackets() {
    assert_eq!(
      url_from_bind("[fe80::1]:3080").as_deref(),
      Some("http://[fe80::1]:3080")
    );
    assert_eq!(
      url_from_bind("fe80::1:3080").as_deref(),
      Some("http://[fe80::1]:3080")
    );
  }

  #[test]
  fn a_bind_without_a_usable_port_is_no_endpoint() {
    assert_eq!(url_from_bind(""), None);
    assert_eq!(url_from_bind("127.0.0.1"), None);
    assert_eq!(url_from_bind("127.0.0.1:"), None);
    assert_eq!(url_from_bind("127.0.0.1:http"), None);
  }

  #[test]
  fn the_flag_outranks_what_the_daemon_reports() {
    // Including when the daemon says the API is off: the operator may be
    // pointing at a different daemon entirely.
    assert_eq!(
      resolve(Some("http://elsewhere:9000/"), Some((false, None))),
      ApiBase::Url("http://elsewhere:9000".to_owned())
    );
  }

  #[test]
  fn the_daemons_own_config_is_used_when_no_flag_is_given() {
    assert_eq!(
      resolve(None, Some((true, Some("0.0.0.0:3080")))),
      ApiBase::Url("http://127.0.0.1:3080".to_owned())
    );
    assert_eq!(
      resolve(None, Some((false, Some("0.0.0.0:3080")))),
      ApiBase::Disabled
    );
    // Enabled but with no bind to speak of is indistinguishable from off, as
    // far as anything this console can do about it.
    assert_eq!(resolve(None, Some((true, None))), ApiBase::Disabled);
    assert_eq!(resolve(None, None), ApiBase::Unknown);
  }
}
