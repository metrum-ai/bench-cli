// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Image generation request building and response decoding shared by
//! `metrum-ai-bench-cli-imagegen` and the strategic `--kind imagegen` sweep.
//! Metrum AI.

use base64::Engine;
use image::GenericImageView;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Fields of one OpenAI `/v1/images/generations` request body.
#[derive(Debug, Clone, Default)]
pub struct GenerationBody<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    pub n: u32,
    pub size: &'a str,
    /// `b64_json` or `url`.
    pub response_format: &'a str,
    pub seed: Option<i64>,
    pub negative_prompt: Option<&'a str>,
    pub num_inference_steps: Option<u32>,
    pub guidance_scale: Option<f64>,
    pub true_cfg_scale: Option<f64>,
    /// Extra top-level fields merged last (they override the fields above).
    pub extra: Option<&'a Map<String, Value>>,
}

impl GenerationBody<'_> {
    /// The JSON body; optional fields are omitted when unset.
    pub fn to_json(&self) -> Value {
        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model));
        body.insert("prompt".to_string(), json!(self.prompt));
        body.insert("n".to_string(), json!(self.n));
        body.insert("size".to_string(), json!(self.size));
        body.insert("response_format".to_string(), json!(self.response_format));
        if let Some(seed) = self.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        if let Some(v) = self.negative_prompt {
            body.insert("negative_prompt".to_string(), json!(v));
        }
        if let Some(v) = self.num_inference_steps {
            body.insert("num_inference_steps".to_string(), json!(v));
        }
        if let Some(v) = self.guidance_scale {
            body.insert("guidance_scale".to_string(), json!(v));
        }
        if let Some(v) = self.true_cfg_scale {
            body.insert("true_cfg_scale".to_string(), json!(v));
        }
        if let Some(extra) = self.extra {
            for (k, v) in extra {
                body.insert(k.clone(), v.clone());
            }
        }
        Value::Object(body)
    }
}

/// Lowercase hex SHA-256 digest.
pub fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// One decoded `b64_json` image.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

/// Error kind (`schema_error` or `decode_error`, as on imagegen records)
/// plus a message.
pub type DecodeError = (&'static str, String);

/// Count the response `data` items and, for `b64_json`, decode each image
/// (base64, then image header) and digest its bytes. `url` responses return
/// the count with no decoded images.
pub fn decode_response_images(
    parsed: &Value,
    b64_json: bool,
) -> Result<(usize, Vec<DecodedImage>), DecodeError> {
    let data = parsed
        .get("data")
        .and_then(Value::as_array)
        .ok_or(("schema_error", "response missing data array".to_string()))?;
    let mut images = Vec::new();
    if b64_json {
        for item in data {
            let b64 = item
                .get("b64_json")
                .and_then(Value::as_str)
                .ok_or(("schema_error", "image item missing b64_json".to_string()))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| ("decode_error", e.to_string()))?;
            let img =
                image::load_from_memory(&bytes).map_err(|e| ("decode_error", e.to_string()))?;
            let (width, height) = img.dimensions();
            images.push(DecodedImage {
                sha256: hex_sha256(&bytes),
                bytes,
                width,
                height,
            });
        }
    }
    Ok((data.len(), images))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 3)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("encode png");
        cursor.into_inner()
    }

    #[test]
    fn body_omits_unset_fields_and_merges_extra() {
        let mut extra = Map::new();
        extra.insert("n".into(), json!(2));
        extra.insert("quality".into(), json!("hd"));
        let body = GenerationBody {
            model: "m",
            prompt: "a cat",
            n: 1,
            size: "64x64",
            response_format: "b64_json",
            seed: Some(7),
            extra: Some(&extra),
            ..Default::default()
        }
        .to_json();
        assert_eq!(body["seed"], 7);
        assert_eq!(body["n"], 2, "extra overrides");
        assert_eq!(body["quality"], "hd");
        assert!(body.get("negative_prompt").is_none());
        assert!(body.get("guidance_scale").is_none());
    }

    #[test]
    fn decodes_b64_images_with_digest_and_size() {
        let png = tiny_png();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let parsed = json!({"data": [{"b64_json": b64}, {"b64_json": b64}]});
        let (count, images) = decode_response_images(&parsed, true).expect("decode");
        assert_eq!(count, 2);
        assert_eq!(images[0].sha256, hex_sha256(&png));
        assert_eq!(images[0].sha256, images[1].sha256);
        assert_eq!((images[0].width, images[0].height), (2, 3));
    }

    #[test]
    fn url_responses_count_without_decoding() {
        let parsed = json!({"data": [{"url": "http://x/1.png"}]});
        let (count, images) = decode_response_images(&parsed, false).expect("count");
        assert_eq!(count, 1);
        assert!(images.is_empty());
    }

    #[test]
    fn decode_errors_are_typed() {
        let missing = decode_response_images(&json!({}), true).unwrap_err();
        assert_eq!(missing.0, "schema_error");
        let bad = decode_response_images(&json!({"data": [{"b64_json": "!!"}]}), true).unwrap_err();
        assert_eq!(bad.0, "decode_error");
    }
}
