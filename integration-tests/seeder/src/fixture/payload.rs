//! Rich JSON payload for healthy-pipeline/ingest GenerateFlowFile.
//!
//! ~600 bytes, multi-line, nested. Every flowfile is unique (two
//! `UUID()` calls plus millisecond timestamp) so provenance search
//! and content-preview views have genuinely distinct data.

/// NiFi Expression Language is evaluated by GenerateFlowFile because we
/// also set the property `Custom Text Length` → empty and use Custom Text
/// mode. See §5.5.1 of the design spec.
pub const HEALTHY_INGEST_CUSTOM_TEXT: &str = r#"{
  "event_id": "${UUID()}",
  "ingested_at": "${now():format('yyyy-MM-dd HH:mm:ss.SSS')}",
  "source": {
    "system": "nifilens-fixture",
    "host": "${hostname()}",
    "pg": "healthy-pipeline/ingest"
  },
  "severity": "INFO",
  "tags": ["synthetic", "fixture", "tracer"],
  "metrics": {
    "latency_ms": 42,
    "bytes_processed": 1024,
    "retries": 0
  },
  "payload": {
    "message": "lorem ipsum dolor sit amet, consectetur adipiscing elit",
    "checksum": "${UUID()}"
  }
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_has_expected_structure() {
        assert!(HEALTHY_INGEST_CUSTOM_TEXT.contains("event_id"));
        assert!(HEALTHY_INGEST_CUSTOM_TEXT.contains("${UUID()}"));
        assert!(HEALTHY_INGEST_CUSTOM_TEXT.contains("checksum"));
        assert!(HEALTHY_INGEST_CUSTOM_TEXT.len() > 400);
    }
}
