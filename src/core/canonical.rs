//! Canonical-form helpers for the generation predicate (spec rev 0.1.4).
//!
//! Verification-side building blocks that keep the *received* bytes authoritative:
//! duplicate-member refusal (I-JSON, RFC 7493), the value-domain gate (integers only,
//! I-JSON safe range), and signature preimages built from the RAW parsed statement so
//! a typed round-trip can never silently drop unknown or malformed members (F4 of the
//! community findings on ossf/tac issue 628).
//!
//! TODO(R4 split): `canonical_json`/`write_canonical` in `provenance.rs` belong here;
//! move them in a dedicated refactor session (no behavior change).

use anyhow::{bail, Context, Result};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// I-JSON safe integer bound (2^53 - 1): above it, JS and Rust parse different values
/// from the same text (JS rounds to the nearest double), so the two implementations
/// would sign different bytes. Findings F6.
pub const MAX_SAFE_INT: i128 = 9_007_199_254_740_991;

/// Parse JSON, refusing DUPLICATE object member names anywhere in the document.
/// `serde_json`'s default is last-wins, which lets a signature cover one reading of a
/// duplicated member while a different parser acts on the other (F4). I-JSON forbids
/// duplicates; so do we.
pub fn parse_json_no_dups(text: &str) -> Result<Value> {
    let mut de = serde_json::Deserializer::from_str(text);
    let v = de
        .deserialize_any(NoDupVisitor)
        .context("parse (duplicate-refusing)")?;
    de.end().context("trailing data after JSON document")?;
    Ok(v)
}

struct NoDupVisitor;

impl<'de> Visitor<'de> for NoDupVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value with no duplicate object member names")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::from(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = vec![];
        while let Some(e) = seq.next_element_seed(NoDupSeed)? {
            out.push(e);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(k) = map.next_key::<String>()? {
            if out.contains_key(&k) {
                return Err(de::Error::custom(format!(
                    "duplicate object member name '{k}' (refused per I-JSON / spec rev 0.1.4)"
                )));
            }
            out.insert(k, map.next_value_seed(NoDupSeed)?);
        }
        Ok(Value::Object(out))
    }
}

struct NoDupSeed;
impl<'de> de::DeserializeSeed<'de> for NoDupSeed {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(NoDupVisitor)
    }
}

/// Enforce the predicate value domain (F6): numbers must be integers within the
/// I-JSON safe range. Floats and out-of-range integers make the two reference
/// implementations hash different bytes, so producers must not emit them and
/// verifiers must refuse them.
pub fn validate_value_domain(v: &Value) -> Result<()> {
    match v {
        Value::Number(n) => {
            if !n.is_i64() && !n.is_u64() {
                bail!("value domain: non-integer number {n} (refused per spec rev 0.1.4)");
            }
            let i = n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from))
                .unwrap();
            if i.abs() > MAX_SAFE_INT {
                bail!("value domain: integer {n} outside the I-JSON safe range (refused per spec rev 0.1.4)");
            }
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(validate_value_domain),
        Value::Object(m) => m.values().try_for_each(validate_value_domain),
        _ => Ok(()),
    }
}

/// Build a signature preimage from the RAW parsed statement (F2/F4).
/// `signoffs_keep = None` removes `predicate.signoffs` entirely (the fab-time
/// statement, which the fab signature and `payload_sha256` cover);
/// `Some(n)` truncates `signoffs` to its first `n` records (the n-th sign-off
/// signature covers the first n records, its own included).
pub fn statement_preimage(raw_statement: &Value, signoffs_keep: Option<usize>) -> Result<Value> {
    let mut s = raw_statement.clone();
    let pred = s
        .get_mut("predicate")
        .and_then(Value::as_object_mut)
        .context("statement has no predicate object")?;
    match signoffs_keep {
        None => {
            pred.remove("signoffs");
        }
        Some(n) => {
            let so = pred
                .get_mut("signoffs")
                .and_then(Value::as_array_mut)
                .context("statement has no signoffs array")?;
            if so.len() < n {
                bail!("signoffs has {} record(s), preimage needs {n}", so.len());
            }
            so.truncate(n);
        }
    }
    Ok(s)
}
