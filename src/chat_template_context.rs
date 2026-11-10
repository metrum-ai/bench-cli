// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Ordered inert template data and explicit HF JSON-filter compatibility subset.

use std::io::{self, Write};

use minijinja::value::{Kwargs, Value};
use minijinja::{Error, ErrorKind};
use serde::Serialize;

fn invalid() -> Error {
    Error::new(
        ErrorKind::InvalidOperation,
        "unsupported HF template JSON operation",
    )
}

fn replace(object: &Value, field: &str, replacement: Value) -> Result<Value, Error> {
    let pairs = object
        .try_iter()?
        .map(|key| {
            let value = if key.as_str() == Some(field) {
                replacement.clone()
            } else {
                object.get_item(&key)?
            };
            Ok((key, value))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Value::from_pairs(pairs))
}

/// Shape validation runs before this ordered representation is built.
pub(super) fn messages(body: &Value) -> Result<Value, Error> {
    let mut messages = Vec::new();
    for message in body.get_attr("messages")?.try_iter()? {
        let calls = message.get_attr("tool_calls")?;
        if calls.is_undefined() || calls.is_none() {
            messages.push(message);
            continue;
        }
        let mut normalized = Vec::new();
        for call in calls.try_iter()? {
            let function = call.get_attr("function")?;
            let arguments = function.get_attr("arguments")?;
            let arguments = match arguments.as_str() {
                Some(text) => serde_json::from_str::<Value>(text).map_err(|_| invalid())?,
                None => arguments,
            };
            normalized.push(replace(
                &call,
                "function",
                replace(&function, "arguments", arguments)?,
            )?);
        }
        messages.push(replace(&message, "tool_calls", Value::from(normalized))?);
    }
    Ok(Value::from(messages))
}

/// Python json.dumps defaults used by HF: spaced delimiters, Unicode retained,
/// no HTML escaping. Preserve insertion order. Nondefault formatting is rejected
/// rather than approximated. Float spellings differ between serializers, so
/// reject floats until a pinned compatible numeric formatter is available.
struct HfFormatter;
impl serde_json::ser::Formatter for HfFormatter {
    fn begin_array_value<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }
    fn write_f64<W: ?Sized + Write>(&mut self, _: &mut W, _: f64) -> io::Result<()> {
        Err(io::Error::other("unsupported floating JSON formatting"))
    }
    fn write_f32<W: ?Sized + Write>(&mut self, _: &mut W, _: f32) -> io::Result<()> {
        Err(io::Error::other("unsupported floating JSON formatting"))
    }
}

pub(super) fn tojson(value: Value, args: Kwargs, max_bytes: usize) -> Result<Value, Error> {
    let ensure_ascii = args.get::<Option<bool>>("ensure_ascii")?.unwrap_or(false);
    let sort_keys = args.get::<Option<bool>>("sort_keys")?.unwrap_or(false);
    let indent = args.get::<Option<Value>>("indent")?;
    let separators = args.get::<Option<Value>>("separators")?;
    args.assert_all_used()?;
    if ensure_ascii
        || sort_keys
        || indent.is_some_and(|v| !v.is_none())
        || separators.is_some_and(|v| !v.is_none())
    {
        return Err(invalid());
    }
    let mut output = super::BoundedOutput {
        bytes: Vec::new(),
        max: max_bytes,
    };
    value
        .serialize(&mut serde_json::Serializer::with_formatter(
            &mut output,
            HfFormatter,
        ))
        .map_err(|_| invalid())?;
    let text = String::from_utf8(output.bytes).map_err(|_| invalid())?;
    Ok(Value::from_safe_string(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hf_json_preserves_order_unicode_spacing_and_html_characters() {
        let raw = r#"{"z":"<>&'é","a":[1,true,null]}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let empty = || std::iter::empty::<(&str, Value)>().collect::<Kwargs>();
        let rendered = tojson(value, empty(), 1024).unwrap();
        assert_eq!(
            rendered.as_str(),
            Some("{\"z\": \"<>&'é\", \"a\": [1, true, null]}")
        );
        assert!(tojson(Value::from(1.25), empty(), 1024).is_err());
        assert!(tojson(Value::from("long"), empty(), 2).is_err());
    }
    #[test]
    fn nondefault_json_options_fail_closed() {
        for (name, value) in [
            ("ensure_ascii", Value::from(true)),
            ("sort_keys", Value::from(true)),
            ("indent", Value::from(2)),
            ("separators", Value::from(vec![",", ":"])),
            ("unrecognized", Value::from(true)),
        ] {
            let args: Kwargs = [(name, value)].into_iter().collect();
            assert!(tojson(Value::from("safe"), args, 100).is_err());
        }
    }
}
