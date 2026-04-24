//! TLS certificate probe: handshakes with each cluster node, captures
//! the presented chain, parses `notAfter` per entry. Surfaces expiry
//! in the Overview Nodes panel + node detail modal.

use std::collections::HashMap;
use std::time::Instant;

use time::OffsetDateTime;

/// Snapshot emitted by the `tls_certs` fetcher — one entry per probed
/// target (cluster node address or, on standalone NiFi, the
/// `ctx.url` host+port). Each value is a per-node `Result` so
/// individual probe failures don't sink the whole snapshot.
#[derive(Debug, Clone)]
pub struct TlsCertsSnapshot {
    pub certs: HashMap<String /* host:port */, Result<NodeCertChain, TlsProbeError>>,
    pub fetched_at: Instant,
    pub fetched_wall: OffsetDateTime,
}

/// Ordered chain as presented by the server: index 0 is the leaf,
/// followed by intermediates. The root is usually absent (servers
/// don't send their trust anchor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCertChain {
    pub entries: Vec<CertEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertEntry {
    pub subject_cn: Option<String>,
    pub not_after: OffsetDateTime,
    pub is_leaf: bool,
}

/// Per-node probe failure. Rendered in the node detail modal; the
/// Nodes-list chip stays silent on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsProbeError {
    Connect(String),
    Handshake(String),
    NoCerts,
    ParseCert(String),
}

impl NodeCertChain {
    /// Minimum `not_after` across every entry in the chain — the date
    /// that actually drives severity. Empty chain returns `None`.
    pub fn earliest_not_after(&self) -> Option<OffsetDateTime> {
        self.entries.iter().map(|e| e.not_after).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn entry(cn: &str, not_after: OffsetDateTime, is_leaf: bool) -> CertEntry {
        CertEntry {
            subject_cn: Some(cn.into()),
            not_after,
            is_leaf,
        }
    }

    #[test]
    fn earliest_not_after_empty_chain_returns_none() {
        let chain = NodeCertChain { entries: vec![] };
        assert_eq!(chain.earliest_not_after(), None);
    }

    #[test]
    fn earliest_not_after_single_entry_returns_that_date() {
        let d = datetime!(2026-05-06 00:00 UTC);
        let chain = NodeCertChain {
            entries: vec![entry("leaf", d, true)],
        };
        assert_eq!(chain.earliest_not_after(), Some(d));
    }

    #[test]
    fn earliest_not_after_returns_min_across_entries() {
        let leaf_exp = datetime!(2028-11-22 00:00 UTC);
        let ca_exp = datetime!(2026-05-06 00:00 UTC);
        let chain = NodeCertChain {
            entries: vec![
                entry("leaf.example", leaf_exp, true),
                entry("Internal Root CA", ca_exp, false),
            ],
        };
        // CA expires first — that's the date we surface.
        assert_eq!(chain.earliest_not_after(), Some(ca_exp));
    }
}
