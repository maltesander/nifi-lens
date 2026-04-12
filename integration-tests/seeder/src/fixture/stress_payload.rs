//! Flat JSON sensor payload for the stress-pipeline GenerateFlowFile.
//!
//! One record per flowfile, flat structure that maps 1:1 to CSV columns.
//! Temperature ranges 0.0-49.9; the RouteOnAttribute threshold of 40
//! gives roughly 80/20 normal/hot split.

/// NiFi Expression Language evaluated by GenerateFlowFile in Custom Text mode.
pub const STRESS_PAYLOAD: &str = r#"{
  "id": "${UUID()}",
  "name": "sensor-${random():mod(1000)}",
  "temperature": ${random():mod(500):divide(10)},
  "humidity": ${random():mod(100)},
  "status": "ok",
  "timestamp": "${now():format('yyyy-MM-dd HH:mm:ss.SSS')}"
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_has_expected_fields() {
        assert!(STRESS_PAYLOAD.contains("\"id\""));
        assert!(STRESS_PAYLOAD.contains("\"temperature\""));
        assert!(STRESS_PAYLOAD.contains("\"humidity\""));
        assert!(STRESS_PAYLOAD.contains("\"status\""));
        assert!(STRESS_PAYLOAD.contains("${UUID()}"));
    }

    #[test]
    fn payload_is_flat_json() {
        // No nested JSON objects — only NiFi EL `${...}` expressions are allowed.
        let inner = &STRESS_PAYLOAD[1..STRESS_PAYLOAD.len() - 1];
        let has_bare_open_brace = inner
            .char_indices()
            .any(|(i, c)| c == '{' && !inner[..i].ends_with('$'));
        assert!(
            !has_bare_open_brace,
            "payload should be flat — no nested JSON objects"
        );
    }
}
