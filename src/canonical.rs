//! Canonical JSON and digest helpers.
//!
//! Canonical form per SOURCE_VERSION_POLICY: lexicographically sorted object keys,
//! UTF-8, no insignificant whitespace, omitted absent optional fields. Every
//! serialization that feeds the wire, the journal, or a digest goes through
//! `serde_json::to_value` first so object keys are emitted in sorted order.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Canonical JSON string for any serializable value (sorted keys, no whitespace).
pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> String {
    // to_value builds serde_json::Map (BTreeMap) objects, which iterate in sorted
    // key order; to_string then emits them canonically.
    let value = serde_json::to_value(value)
        .expect("canonical serialization of a runtime value cannot fail");
    serde_json::to_string(&value).expect("canonical JSON emission cannot fail")
}

/// Normalize every JSON number to serde's canonical exact representation.
///
/// Integer tokens must fit serde's exact signed/unsigned range. Floating-point
/// tokens must decode to a finite `f64`. Returns false without promising a
/// complete normalization when any token is outside that shared contract.
pub fn normalize_shared_numbers(value: &mut Value) -> bool {
    match value {
        Value::Number(number) => {
            let text = number.to_string();
            let normalized = if text.contains(['.', 'e', 'E']) {
                number
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .and_then(serde_json::Number::from_f64)
            } else if let Some(value) = number.as_i64() {
                Some(serde_json::Number::from(value))
            } else {
                number.as_u64().map(serde_json::Number::from)
            };
            if let Some(normalized) = normalized {
                *number = normalized;
                true
            } else {
                false
            }
        }
        Value::Array(values) => values.iter_mut().all(normalize_shared_numbers),
        Value::Object(values) => values.values_mut().all(normalize_shared_numbers),
        _ => true,
    }
}

pub fn has_only_shared_numbers(value: &Value) -> bool {
    normalize_shared_numbers(&mut value.clone())
}

/// Canonical JSON bytes for any serializable value.
pub fn canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    canonical_json(value).into_bytes()
}

/// Hex sha256 of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

/// Hex sha256 of the canonical JSON form of a value.
pub fn digest_json<T: Serialize + ?Sized>(value: &T) -> String {
    sha256_hex(&canonical_bytes(value))
}

/// Hex sha256 of the concatenation of several byte slices.
pub fn digest_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Sample {
        b: u64,
        a: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        opt: Option<u64>,
    }

    #[test]
    fn keys_are_sorted_and_absent_fields_omitted() {
        let s = Sample {
            b: 2,
            a: "x".into(),
            opt: None,
        };
        assert_eq!(canonical_json(&s), r#"{"a":"x","b":2}"#);
    }

    #[test]
    fn digest_is_stable_hex_sha256() {
        let d1 = digest_json(&Sample {
            b: 1,
            a: "y".into(),
            opt: Some(3),
        });
        let d2 = digest_json(&Sample {
            b: 1,
            a: "y".into(),
            opt: Some(3),
        });
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64);
        assert_eq!(
            digest_json(&serde_json::json!({"a": 1})),
            "015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862"
        );
    }

    #[test]
    fn float_notation_matches_cross_language_contract() {
        assert_eq!(
            canonical_json(&serde_json::json!({
                "float_boundaries": [1e-5, 1e-6, 1e-7, 1e16, 1e20, -0.0]
            })),
            r#"{"float_boundaries":[0.00001,1e-6,1e-7,1e+16,1e+20,-0.0]}"#
        );
    }

    #[test]
    fn shared_number_range_rejects_unrepresentable_integer_tokens() {
        let valid: serde_json::Value =
            serde_json::from_str(r#"[-9223372036854775808,18446744073709551615,1.5,1e100]"#)
                .unwrap();
        assert!(has_only_shared_numbers(&valid));

        for text in ["-9223372036854775809", "18446744073709551616"] {
            let value: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(!has_only_shared_numbers(&value));
        }
    }
}
