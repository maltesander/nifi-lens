//! TLS certificate probe: handshakes with each cluster node, captures
//! the presented chain, parses `notAfter` per entry. Surfaces expiry
//! in the Overview Nodes panel + node detail modal.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
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

/// Probe a single TLS endpoint. Returns the server's presented chain
/// with per-entry `CertEntry` parsed. Uses a permissive verifier: we
/// are not authenticating, only observing.
///
/// The chain is captured from inside the verifier callback — this
/// works even when the handshake later fails (e.g. the server demands
/// a client cert we don't present). Any rustls state past the
/// verifier call is treated as best-effort.
pub async fn probe_tls(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<NodeCertChain, TlsProbeError> {
    let addr = format!("{host}:{port}");
    let tcp = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr))
        .await
        .map_err(|_| TlsProbeError::Connect(format!("connect timeout after {timeout:?}")))?
        .map_err(|e| TlsProbeError::Connect(e.to_string()))?;

    let captured: Arc<Mutex<Option<Vec<Vec<u8>>>>> = Arc::new(Mutex::new(None));
    let verifier = Arc::new(CaptureVerifier {
        sink: captured.clone(),
    });

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| TlsProbeError::Handshake(format!("invalid server name: {e}")))?;

    // Drive the handshake under the same timeout budget. Whether this
    // returns Ok or Err is secondary — the verifier callback has
    // already captured the chain if the server sent one.
    let _ = tokio::time::timeout(timeout, connector.connect(server_name, tcp)).await;

    let captured = captured
        .lock()
        .map_err(|e| TlsProbeError::Handshake(format!("verifier mutex poisoned: {e}")))?
        .take();

    let Some(raw) = captured else {
        return Err(TlsProbeError::Handshake(
            "server sent no certificate chain".into(),
        ));
    };
    if raw.is_empty() {
        return Err(TlsProbeError::NoCerts);
    }

    let mut entries = Vec::with_capacity(raw.len());
    for (i, der_bytes) in raw.iter().enumerate() {
        let cert_der = rustls::pki_types::CertificateDer::from(der_bytes.as_slice());
        entries.push(parse_cert_der(&cert_der, i == 0)?);
    }
    Ok(NodeCertChain { entries })
}

#[derive(Debug)]
struct CaptureVerifier {
    sink: Arc<Mutex<Option<Vec<Vec<u8>>>>>,
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut chain = Vec::with_capacity(1 + intermediates.len());
        chain.push(end_entity.to_vec());
        for i in intermediates {
            chain.push(i.to_vec());
        }
        if let Ok(mut guard) = self.sink.lock() {
            *guard = Some(chain);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

impl NodeCertChain {
    /// Minimum `not_after` across every entry in the chain — the date
    /// that actually drives severity. Empty chain returns `None`.
    pub fn earliest_not_after(&self) -> Option<OffsetDateTime> {
        self.entries.iter().map(|e| e.not_after).min()
    }
}

/// Probe every target concurrently. Each entry is `Ok` or an
/// individual `TlsProbeError` — the fetcher never aborts the whole
/// snapshot on a single node's failure. Takes `host:port` strings;
/// each is split on the rightmost `:` (v4-safe; IPv6 callers should
/// wrap the host in brackets).
pub async fn probe_all(targets: &[String], timeout: Duration) -> TlsCertsSnapshot {
    let fetched_at = Instant::now();
    let fetched_wall = OffsetDateTime::now_utc();

    let futures = targets.iter().map(|t| {
        let target = t.clone();
        async move {
            let result = match split_host_port(&target) {
                Some((host, port)) => probe_tls(&host, port, timeout).await,
                None => Err(TlsProbeError::Connect(format!("bad target {target}"))),
            };
            (target, result)
        }
    });

    let results: Vec<(String, Result<NodeCertChain, TlsProbeError>)> =
        futures::future::join_all(futures).await;
    let certs = results.into_iter().collect();
    TlsCertsSnapshot {
        certs,
        fetched_at,
        fetched_wall,
    }
}

fn split_host_port(target: &str) -> Option<(String, u16)> {
    let idx = target.rfind(':')?;
    let (host, port_str) = target.split_at(idx);
    let port: u16 = port_str[1..].parse().ok()?;
    Some((host.to_string(), port))
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
mod probe_tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};
    use rustls::ServerConfig;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use std::sync::Arc;
    use std::time::Duration;
    use time::OffsetDateTime;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    /// Start an in-process rustls TLS server on a loopback port. Returns
    /// the bound port and the `not_after` of the cert so tests can assert
    /// against it.
    async fn start_test_server() -> (u16, OffsetDateTime) {
        let nb = OffsetDateTime::now_utc() - time::Duration::hours(1);
        let na = OffsetDateTime::now_utc() + time::Duration::hours(1);

        let mut params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "localhost");
        params.not_before = nb;
        params.not_after = na;
        let kp = KeyPair::generate().unwrap();
        let cert = params.self_signed(&kp).unwrap();

        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(kp.serialize_der()));

        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // Best-effort; probe drops the connection on success.
                    let _ = acceptor.accept(stream).await;
                });
            }
        });

        (port, na)
    }

    #[tokio::test]
    async fn probe_tls_returns_chain_with_expected_not_after() {
        let (port, expected_na) = start_test_server().await;
        let chain = probe_tls("127.0.0.1", port, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(chain.entries.len(), 1);
        assert!(chain.entries[0].is_leaf);
        assert_eq!(chain.entries[0].subject_cn.as_deref(), Some("localhost"));
        // rcgen truncates to second precision; compare whole seconds.
        let delta = (chain.entries[0].not_after - expected_na)
            .whole_seconds()
            .abs();
        assert!(delta <= 1, "not_after drifted by {delta}s");
    }

    #[tokio::test]
    async fn probe_tls_unbound_port_returns_connect_error() {
        // Bind then drop to get a port that's guaranteed unbound.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let err = probe_tls("127.0.0.1", port, Duration::from_millis(500))
            .await
            .unwrap_err();
        assert!(matches!(err, TlsProbeError::Connect(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn probe_tls_plain_tcp_server_returns_handshake_error() {
        // Accept TCP but never TLS.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((_stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        let err = probe_tls("127.0.0.1", port, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(matches!(err, TlsProbeError::Handshake(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn probe_all_fans_out_and_keys_by_target() {
        let (port_ok, _) = start_test_server().await;
        // An unbound port for the failure case.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_bad = l.local_addr().unwrap().port();
        drop(l);

        let targets = vec![
            format!("127.0.0.1:{port_ok}"),
            format!("127.0.0.1:{port_bad}"),
        ];
        let snap = probe_all(&targets, Duration::from_millis(500)).await;

        assert_eq!(snap.certs.len(), 2);
        assert!(snap.certs[&targets[0]].is_ok());
        assert!(matches!(
            snap.certs[&targets[1]],
            Err(TlsProbeError::Connect(_))
        ));
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
