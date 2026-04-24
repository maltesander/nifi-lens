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

/// Parse one DER-encoded certificate into a `CertEntry`. The caller
/// asserts whether this entry is the leaf (chain[0]) or an
/// intermediate (chain[1..]).
#[allow(dead_code)] // used by Task 4: probe_tls
pub(crate) fn parse_cert_der(
    der: &rustls::pki_types::CertificateDer<'_>,
    is_leaf: bool,
) -> Result<CertEntry, TlsProbeError> {
    let (_, parsed) = x509_parser::parse_x509_certificate(der.as_ref())
        .map_err(|e| TlsProbeError::ParseCert(e.to_string()))?;

    // `validity().not_after` is an `ASN1Time`; `to_datetime()` returns
    // the `time::OffsetDateTime` we need directly.
    let not_after = parsed.validity().not_after.to_datetime();

    // Pull a single CN attribute. SAN-only certs omit CN entirely —
    // tolerate that gracefully.
    let subject_cn = parsed
        .subject()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .map(|s| s.to_string());

    Ok(CertEntry {
        subject_cn,
        not_after,
        is_leaf,
    })
}

impl NodeCertChain {
    /// Minimum `not_after` across every entry in the chain — the date
    /// that actually drives severity. Empty chain returns `None`.
    pub fn earliest_not_after(&self) -> Option<OffsetDateTime> {
        self.entries.iter().map(|e| e.not_after).min()
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};
    use rustls::pki_types::CertificateDer;
    use time::macros::datetime;

    fn make_leaf_der(cn: &str, not_before: OffsetDateTime, not_after: OffsetDateTime) -> Vec<u8> {
        let mut params = CertificateParams::new(vec![cn.to_string()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, cn);
        params.not_before = not_before;
        params.not_after = not_after;
        let kp = KeyPair::generate().unwrap();
        let cert = params.self_signed(&kp).unwrap();
        cert.der().to_vec()
    }

    #[test]
    fn parse_cert_der_extracts_cn_and_not_after() {
        let nb = datetime!(2026-01-01 00:00 UTC);
        let na = datetime!(2027-01-01 00:00 UTC);
        let der = make_leaf_der("node1.nifi.local", nb, na);
        let cert_der = CertificateDer::from(der.as_slice());

        let entry = parse_cert_der(&cert_der, true).unwrap();

        assert_eq!(entry.subject_cn.as_deref(), Some("node1.nifi.local"));
        assert_eq!(entry.not_after, na);
        assert!(entry.is_leaf);
    }

    #[test]
    fn parse_cert_der_malformed_bytes_returns_parse_error() {
        let garbage = [0xFFu8; 16];
        let cert_der = CertificateDer::from(garbage.as_slice());
        let err = parse_cert_der(&cert_der, false).unwrap_err();
        assert!(matches!(err, TlsProbeError::ParseCert(_)));
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
