//! Draft scene document (contract version 1) validation.
//!
//! The engine validates every scene independently of the management plane
//! (docs/MASTER-ARCHITECTURE.md §17, §22). The rules mirror
//! `contracts/scene.schema.json` plus semantic checks a JSON Schema cannot
//! express (unique layer ids, layers fully inside the canvas). The same
//! fixtures in `contracts/fixtures/scenes` are exercised by the Rust and Ruby
//! test suites so the two validators cannot silently diverge.

use serde_json::{Map, Value};

pub const CANVAS_WIDTH: i64 = 1920;
pub const CANVAS_HEIGHT: i64 = 1080;
pub const MAX_LAYERS: usize = 16;
pub const MAX_NAME_CHARS: usize = 100;
/// Source identifiers accepted by draft version 1. `hdmi` is the legacy draft
/// spelling of the future `hdmi:0` identifier; changing it requires a
/// versioned migration, not a silent rename.
pub const SOURCES_V1: [&str; 3] = ["hdmi", "phone:rear", "phone:front"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneError {
    /// JSON-pointer-like location of the first violation.
    pub path: String,
    pub reason: &'static str,
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

fn err(path: impl Into<String>, reason: &'static str) -> SceneError {
    SceneError { path: path.into(), reason }
}

fn exact_keys(object: &Map<String, Value>, keys: &[&str], path: &str) -> Result<(), SceneError> {
    if object.len() != keys.len() || !keys.iter().all(|k| object.contains_key(*k)) {
        return Err(err(path, "unexpected_or_missing_property"));
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn integer(layer: &Map<String, Value>, key: &str, path: &str) -> Result<i64, SceneError> {
    // serde_json keeps 1.0 as a float; only true JSON integers are accepted,
    // matching Ruby's `Integer` check and the schema's "integer" type.
    match layer.get(key) {
        Some(Value::Number(n)) if n.is_i64() => Ok(n.as_i64().unwrap_or_default()),
        _ => Err(err(format!("{path}/{key}"), "not_integer")),
    }
}

/// Validate a draft scene document of contract version 1.
pub fn validate_draft_v1(doc: &Value) -> Result<(), SceneError> {
    let Value::Object(root) = doc else { return Err(err("", "not_object")) };
    exact_keys(root, &["version", "name", "layers"], "")?;
    if root.get("version") != Some(&Value::from(1)) {
        return Err(err("/version", "unsupported_version"));
    }
    match root.get("name") {
        Some(Value::String(name)) if (1..=MAX_NAME_CHARS).contains(&name.chars().count()) => {}
        _ => return Err(err("/name", "invalid_name")),
    }
    let Some(Value::Array(layers)) = root.get("layers") else { return Err(err("/layers", "not_array")) };
    if layers.len() > MAX_LAYERS {
        return Err(err("/layers", "too_many_layers"));
    }
    let mut seen: Vec<&str> = Vec::with_capacity(layers.len());
    for (index, layer) in layers.iter().enumerate() {
        let path = format!("/layers/{index}");
        let Value::Object(layer) = layer else { return Err(err(path, "not_object")) };
        exact_keys(layer, &["id", "source", "x", "y", "width", "height", "visible"], &path)?;
        let id = match layer.get("id") {
            Some(Value::String(id)) if valid_id(id) => id.as_str(),
            _ => return Err(err(format!("{path}/id"), "invalid_id")),
        };
        if seen.contains(&id) {
            return Err(err(format!("{path}/id"), "duplicate_id"));
        }
        seen.push(id);
        match layer.get("source") {
            Some(Value::String(s)) if SOURCES_V1.contains(&s.as_str()) => {}
            _ => return Err(err(format!("{path}/source"), "unknown_source")),
        }
        if !matches!(layer.get("visible"), Some(Value::Bool(_))) {
            return Err(err(format!("{path}/visible"), "not_boolean"));
        }
        let x = integer(layer, "x", &path)?;
        let y = integer(layer, "y", &path)?;
        let w = integer(layer, "width", &path)?;
        let h = integer(layer, "height", &path)?;
        if x < 0 || y < 0 || w <= 0 || h <= 0 {
            return Err(err(path, "negative_or_empty_geometry"));
        }
        if x.saturating_add(w) > CANVAS_WIDTH || y.saturating_add(h) > CANVAS_HEIGHT {
            return Err(err(path, "outside_canvas"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn fixtures(kind: &str) -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/fixtures/scenes").join(kind);
        let mut out: Vec<(String, Value)> = fs::read_dir(&dir).expect("fixture dir").flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| {
                let text = fs::read_to_string(e.path()).expect("fixture");
                (e.file_name().to_string_lossy().into_owned(), serde_json::from_str(&text).expect("fixture json"))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        assert!(!out.is_empty(), "no {kind} fixtures");
        out
    }

    #[test]
    fn shared_valid_fixtures_pass() {
        for (name, doc) in fixtures("valid") {
            assert_eq!(validate_draft_v1(&doc), Ok(()), "{name}");
        }
    }

    #[test]
    fn shared_invalid_fixtures_fail() {
        for (name, doc) in fixtures("invalid") {
            assert!(validate_draft_v1(&doc).is_err(), "{name} unexpectedly valid");
        }
    }

    #[test]
    fn float_geometry_is_rejected() {
        let doc: Value = serde_json::from_str(
            r#"{"version":1,"name":"a","layers":[{"id":"a","source":"hdmi","x":0.0,"y":0,"width":10,"height":10,"visible":true}]}"#,
        ).unwrap();
        assert_eq!(validate_draft_v1(&doc).unwrap_err().reason, "not_integer");
    }

    #[test]
    fn huge_integers_do_not_overflow() {
        let doc: Value = serde_json::from_str(&format!(
            r#"{{"version":1,"name":"a","layers":[{{"id":"a","source":"hdmi","x":{},"y":0,"width":{},"height":10,"visible":true}}]}}"#,
            i64::MAX, i64::MAX
        )).unwrap();
        assert!(validate_draft_v1(&doc).is_err());
    }
}
