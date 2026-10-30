// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared request body overrides for Metrum AI Bench.

use serde_json::{Map, Value};

/// Merge top-level overrides. Null removes a key; nested objects are not merged.
pub fn merge_extra_body(
    base: &mut Map<String, Value>,
    extra: &Value,
) -> Result<(), std::io::Error> {
    let extra = extra.as_object().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--extra-body-json must be a JSON object",
        )
    })?;
    for (key, value) in extra {
        if value.is_null() {
            base.remove(key);
        } else {
            base.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overrides_remove_and_validate() {
        let mut base = json!({"a": 1, "nested": {"old": 1}})
            .as_object()
            .unwrap()
            .clone();
        merge_extra_body(
            &mut base,
            &json!({"a": null, "absent": null, "nested": {"x": null}, "b": 2}),
        )
        .unwrap();
        assert_eq!(
            Value::Object(base.clone()),
            json!({"nested": {"x": null}, "b": 2})
        );
        assert!(merge_extra_body(&mut base, &json!([1, 2])).is_err());
        assert!(merge_extra_body(&mut base, &json!("x")).is_err());
    }
}
