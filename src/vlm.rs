// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! VLM request building shared by `metrum-ai-bench-cli-vlm` and the strategic
//! `--kind vlm` sweep: image loading (bounded LRU cache), OpenAI `image_url`
//! content parts, and the chat completions body. Metrum AI.

use base64::Engine;
use log::debug;
use reqwest::Client;
use serde_json::{json, Value};
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

/// Loaded image bytes (base64) plus the metadata recorded per request.
/// `base64_data` is shared, so a cache hit or a clone never copies the
/// payload (#242).
#[derive(Clone)]
pub struct ImageData {
    pub base64_data: Arc<str>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    pub url: String, // Add this field to store the original URL
}

/// Bounded LRU image cache (HashMap + VecDeque; no third-party lru crate).
pub struct ImageCache {
    map: std::collections::HashMap<String, ImageData>,
    order: std::collections::VecDeque<String>,
    capacity: usize,
}

impl ImageCache {
    pub fn new(capacity: usize) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if capacity == 0 {
            return Err("image_cache_size must be >= 1".into());
        }
        Ok(Self {
            map: std::collections::HashMap::with_capacity(capacity),
            order: std::collections::VecDeque::with_capacity(capacity),
            capacity,
        })
    }

    pub fn get(&mut self, key: &str) -> Option<ImageData> {
        if !self.map.contains_key(key) {
            return None;
        }
        if let Some(k) = self
            .order
            .iter()
            .position(|k| k == key)
            .and_then(|pos| self.order.remove(pos))
        {
            self.order.push_back(k);
        }
        self.map.get(key).cloned()
    }

    pub fn put(&mut self, key: String, value: ImageData) {
        if self.map.contains_key(&key) {
            self.order.retain(|k| k != &key);
        } else if self.map.len() >= self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    pub async fn get_or_load(
        &mut self,
        client: &Client,
        path: &str,
        max_dimension: Option<u32>,
        timeout_secs: u64,
        reencode_jpeg: bool,
    ) -> Result<ImageData, Box<dyn Error + Send + Sync>> {
        // `data:` URLs are keyed (and logged) by digest so large inline images
        // are not duplicated into the cache key or every log line.
        let key = crate::prompt_inputs::image_ref_key(path);
        let label = key.as_str();
        if let Some(data) = self.get(&key) {
            debug!("Cache hit for image: {}", label);
            return Ok(data);
        }

        debug!("Cache miss for image: {}", label);
        let mut declared_mime = None;
        // Load and process image
        let image_data = if crate::prompt_inputs::is_data_url(path) {
            // Decoding a large inline image is CPU work: blocking pool (#242).
            let inline = path.to_string();
            let decoded =
                tokio::task::spawn_blocking(move || crate::prompt_inputs::decode_data_url(&inline))
                    .await
                    .map_err(|e| format!("image decode task failed for '{}': {}", label, e))??;
            debug!(
                "Decoded inline image {} ({} bytes)",
                label,
                decoded.bytes.len()
            );
            declared_mime = Some(decoded.declared_mime);
            decoded.bytes
        } else if path.starts_with("http://") || path.starts_with("https://") {
            // For URLs, use client with timeout (no unbounded reqwest::get)
            debug!("Fetching image from URL: {}", path);
            let response = client
                .get(path)
                .timeout(Duration::from_secs(timeout_secs))
                .send()
                .await
                .map_err(|e| format!("Failed to fetch image from URL '{}': {}", path, e))?;
            if !response.status().is_success() {
                return Err(format!(
                    "Failed to fetch image from URL '{}': HTTP {}",
                    path,
                    response.status()
                )
                .into());
            }
            response
                .bytes()
                .await
                .map_err(|e| format!("Failed to read image data from URL '{}': {}", path, e))?
                .to_vec()
        } else {
            // For local files, use fs::read
            debug!("Loading image from local file: {}", path);
            tokio::fs::read(path)
                .await
                .map_err(|e| format!("Failed to read local image file '{}': {}", path, e))?
        };

        // Header parse, optional resize / re-encode and base64 are CPU work
        // on large inputs: run them off the async worker threads (#242).
        let label_owned = label.to_string();
        let (base64_data, mime_type, width, height, size_bytes) =
            tokio::task::spawn_blocking(move || {
                encode_image(
                    image_data,
                    declared_mime,
                    &label_owned,
                    max_dimension,
                    reencode_jpeg,
                )
            })
            .await
            .map_err(|e| format!("image processing task failed for '{}': {}", label, e))??;

        let image_data = ImageData {
            base64_data,
            mime_type,
            width,
            height,
            size_bytes,
            // Original URL/path for server_side_download; data: URLs keep only
            // their digest (server_side_download never forwards them).
            url: key.clone(),
        };

        self.put(key, image_data.clone());
        Ok(image_data)
    }
}

/// Detect the MIME type, read the dimensions, optionally resize or
/// re-encode, and base64 the payload. Blocking CPU work: callers run it on
/// the blocking pool. Returns (base64, mime, width, height, payload bytes).
#[allow(clippy::type_complexity)]
fn encode_image(
    image_data: Vec<u8>,
    declared_mime: Option<String>,
    label: &str,
    max_dimension: Option<u32>,
    reencode_jpeg: bool,
) -> Result<(Arc<str>, String, u32, u32, u64), Box<dyn Error + Send + Sync>> {
    let detected_format = image::guess_format(&image_data).ok();
    let mut mime_type = match detected_format {
        Some(image::ImageFormat::Png) => "image/png",
        Some(image::ImageFormat::Gif) => "image/gif",
        Some(image::ImageFormat::WebP) => "image/webp",
        _ => "image/jpeg",
    }
    .to_string();
    if let Some(declared) = declared_mime.filter(|d| !d.is_empty() && *d != mime_type) {
        debug!(
            "data: URL {} declares {} but the bytes look like {}; sending {}",
            label, declared, mime_type, mime_type
        );
    }

    // Read the header for dimensions; the payload stays byte-identical to
    // the source unless a resize or an explicit re-encode is requested.
    let (source_width, source_height) = image::ImageReader::new(std::io::Cursor::new(&image_data))
        .with_guessed_format()
        .map_err(|e| format!("Failed to read image header from '{}': {}", label, e))?
        .into_dimensions()
        .map_err(|e| format!("Failed to read image size from '{}': {}", label, e))?;

    // The limit, only when the image exceeds it.
    let resize_to =
        max_dimension.filter(|max_dim| source_width > *max_dim || source_height > *max_dim);
    let oversized = resize_to.is_some();

    let (encoded, width, height) = if oversized || reencode_jpeg {
        let mut img = image::load_from_memory(&image_data)
            .map_err(|e| format!("Failed to decode image from '{}': {}", label, e))?;
        if let Some(max_dim) = resize_to {
            let scale = max_dim as f32 / source_width.max(source_height) as f32;
            let new_width = (source_width as f32 * scale) as u32;
            let new_height = (source_height as f32 * scale) as u32;
            img = img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3);
            debug!(
                "Resized image from {}x{} to {}x{}",
                source_width, source_height, new_width, new_height
            );
        }
        let format = if reencode_jpeg {
            image::ImageFormat::Jpeg
        } else {
            image::ImageFormat::Png
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        // JPEG cannot store alpha; drop it rather than failing the request.
        if format == image::ImageFormat::Jpeg {
            image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut cursor, format)?;
        } else {
            img.write_to(&mut cursor, format)?;
        }
        mime_type = match format {
            image::ImageFormat::Jpeg => "image/jpeg",
            _ => "image/png",
        }
        .to_string();
        let dimensions = (img.width(), img.height());
        (cursor.into_inner(), dimensions.0, dimensions.1)
    } else {
        (image_data, source_width, source_height)
    };

    let base64_data = base64::engine::general_purpose::STANDARD.encode(&encoded);
    let size_bytes = encoded.len() as u64;
    Ok((Arc::from(base64_data), mime_type, width, height, size_bytes))
}

/// Options for one `image_url` content part.
#[derive(Debug, Clone)]
pub struct ImageContentOptions {
    pub detail: String,
    pub server_side_download: bool,
    // Add more options here as needed
    // format: String,
    // quality: u8,
    // etc.
}

impl Default for ImageContentOptions {
    fn default() -> Self {
        Self {
            detail: "low".to_string(),
            server_side_download: false,
        }
    }
}

/// Format one image as an OpenAI `image_url` content part (base64 data URL,
/// or the original http(s) URL with `server_side_download`).
pub fn format_image_content(image: &ImageData, options: &ImageContentOptions) -> Value {
    let image_url = if options.server_side_download {
        // If server-side download is enabled, use the original URL
        // Note: This assumes the ImageData struct has a url field
        // We'll need to modify the ImageData struct to store the original URL
        image.url.clone()
    } else {
        // Otherwise, use base64 encoded data
        format!("data:{};base64,{}", image.mime_type, image.base64_data)
    };

    json!({
        "type": "image_url",
        "image_url": {
            "url": image_url,
            "detail": options.detail
        }
    })
}

/// Build the OpenAI chat completions body for a VLM request: optional system
/// message, then one user message with the text prompt and image parts.
#[allow(clippy::too_many_arguments)]
pub fn build_request_body(
    model: &str,
    max_tokens: u32,
    temperature: f32,
    prompt: &str,
    images: &[ImageData],
    image_detail: &str,
    server_side_download: bool,
    streaming: bool,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    extra_body_json: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    let system =
        system_prompt.unwrap_or("You are a helpful assistant capable of understanding images.");
    let mut messages = Vec::new();

    if !system.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": system
        }));
    }

    let mut content = Vec::new();

    if !prompt.is_empty() {
        content.push(json!({
            "type": "text",
            "text": prompt
        }));
    }

    // Create image content options
    let image_options = ImageContentOptions {
        detail: image_detail.to_string(),
        server_side_download,
        // Add more options here as needed
    };

    // Format each image with the options
    for image in images {
        content.push(format_image_content(image, &image_options));
    }

    messages.push(json!({
        "role": "user",
        "content": content
    }));

    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": streaming
    });
    if ignore_eos {
        body["ignore_eos"] = json!(true);
    }
    if let Some(min_t) = min_tokens {
        body["min_tokens"] = json!(min_t);
    }
    if streaming {
        body["stream_options"] = json!({"include_usage": true});
    }
    if let Some(extra) = extra_body_json {
        let extra_val: Value = serde_json::from_str(extra)?;
        if let (Some(base), Some(extra_map)) = (body.as_object_mut(), extra_val.as_object()) {
            for (k, v) in extra_map {
                base.insert(k.clone(), v.clone());
            }
        }
    }

    // Pretty-printing a body with base64 images is costly; only when logged.
    if log::log_enabled!(log::Level::Debug) {
        let body_str = serde_json::to_string_pretty(&body).unwrap_or_default();
        if body_str.contains("base64") {
            debug!("Built request body: (redacted: contains image data)");
        } else {
            debug!("Built request body: {}", body_str);
        }
    }
    Ok(body)
}

/// Inline base64 bytes across `images`: callers build bodies above a size
/// threshold on the blocking pool.
pub fn inline_image_bytes(images: &[ImageData]) -> usize {
    images.iter().map(|image| image.base64_data.len()).sum()
}

/// Placeholder written in place of image `index`'s base64 in the skeleton
/// body. The control characters serialize as `\u0001`, which no base64 text
/// or JSON-escaped prompt text can produce by accident.
fn image_placeholder(index: usize) -> String {
    format!("\u{1}metrum-ai-image-{index}\u{1}")
}

/// Build and serialize a VLM chat body to the bytes sent on the wire (#242).
/// Callers do this before taking the send time, so JSON encoding of a large
/// base64 payload never lands in `latency_s`, TTFT or `t_sent_ns`. The
/// output is byte-identical to `serde_json::to_vec(&build_request_body(..))`,
/// but each image's base64 is copied once, straight from the shared cache
/// entry into the body: the JSON skeleton carries a short placeholder that
/// is replaced while writing the final buffer. Base64 and the image MIME
/// types need no JSON escaping.
#[allow(clippy::too_many_arguments)]
pub fn build_request_bytes(
    model: &str,
    max_tokens: u32,
    temperature: f32,
    prompt: &str,
    images: &[ImageData],
    image_detail: &str,
    server_side_download: bool,
    streaming: bool,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    extra_body_json: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<bytes::Bytes, Box<dyn Error + Send + Sync>> {
    let inline = !server_side_download && inline_image_bytes(images) > 0;
    let skeleton_images: Vec<ImageData> = if inline {
        images
            .iter()
            .enumerate()
            .map(|(index, image)| ImageData {
                base64_data: Arc::from(image_placeholder(index)),
                ..image.clone()
            })
            .collect()
    } else {
        images.to_vec()
    };
    let body = build_request_body(
        model,
        max_tokens,
        temperature,
        prompt,
        &skeleton_images,
        image_detail,
        server_side_download,
        streaming,
        ignore_eos,
        min_tokens,
        extra_body_json,
        system_prompt,
    )?;
    let skeleton = serde_json::to_vec(&body)?;
    if !inline {
        return Ok(bytes::Bytes::from(skeleton));
    }
    // JSON form of each placeholder, without the quotes (it sits inside the
    // url string), and where it appears in the skeleton.
    let tokens = (0..images.len())
        .map(|index| {
            serde_json::to_string(&image_placeholder(index))
                .map(|quoted| quoted[1..quoted.len() - 1].to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let spans: Option<Vec<usize>> = tokens
        .iter()
        .map(|token| {
            let mut hits = skeleton
                .windows(token.len())
                .enumerate()
                .filter(|(_, window)| *window == token.as_bytes())
                .map(|(at, _)| at);
            // Exactly one hit, or the prompt or extra body carries the same
            // text and the splice could land in the wrong place.
            match (hits.next(), hits.next()) {
                (Some(at), None) => Some(at),
                _ => None,
            }
        })
        .collect();
    let Some(spans) = spans.filter(|spans| spans.windows(2).all(|w| w[0] < w[1])) else {
        // Fall back to plain serialization (one extra copy, same bytes).
        let body = build_request_body(
            model,
            max_tokens,
            temperature,
            prompt,
            images,
            image_detail,
            server_side_download,
            streaming,
            ignore_eos,
            min_tokens,
            extra_body_json,
            system_prompt,
        )?;
        return Ok(bytes::Bytes::from(serde_json::to_vec(&body)?));
    };
    let mut out = Vec::with_capacity(skeleton.len() + inline_image_bytes(images));
    let mut copied = 0;
    for ((image, token), at) in images.iter().zip(&tokens).zip(spans) {
        out.extend_from_slice(&skeleton[copied..at]);
        out.extend_from_slice(image.base64_data.as_bytes());
        copied = at + token.len();
    }
    out.extend_from_slice(&skeleton[copied..]);
    Ok(bytes::Bytes::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(url: &str, bytes: u64) -> ImageData {
        ImageData {
            base64_data: "QUJD".into(),
            mime_type: "image/png".into(),
            width: 2,
            height: 2,
            size_bytes: bytes,
            url: url.into(),
        }
    }

    #[test]
    fn request_bytes_match_the_serialized_json_body() {
        let mut second = image("b.png", 4);
        second.base64_data = Arc::from("iVBORw0KGgo+/=");
        second.mime_type = "image/jpeg".into();
        let images = [image("a.png", 3), second];
        let shared = images[0].clone();
        assert!(Arc::ptr_eq(&shared.base64_data, &images[0].base64_data));
        // A prompt that looks like a placeholder must survive unchanged.
        let prompt = "describe metrum-ai-image-0 \"quoted\" \u{1}";
        let colliding = format!("x {}", image_placeholder(0));
        for (prompt, ssd, extra, system) in [
            (prompt, false, None, None),
            (prompt, false, Some(r#"{"top_p":0.9}"#), Some("sys")),
            (prompt, true, None, Some("")),
            (colliding.as_str(), false, None, None),
        ] {
            let body = build_request_body(
                "m",
                8,
                0.1,
                prompt,
                &images,
                "high",
                ssd,
                true,
                true,
                Some(2),
                extra,
                system,
            )
            .expect("body");
            let bytes = build_request_bytes(
                "m",
                8,
                0.1,
                prompt,
                &images,
                "high",
                ssd,
                true,
                true,
                Some(2),
                extra,
                system,
            )
            .expect("bytes");
            assert_eq!(bytes.as_ref(), serde_json::to_vec(&body).expect("json"));
        }
        let none = build_request_bytes(
            "m",
            8,
            0.1,
            "x",
            &[],
            "low",
            false,
            false,
            false,
            None,
            None,
            None,
        )
        .expect("no images");
        let parsed: Value = serde_json::from_slice(&none).expect("parse");
        assert_eq!(parsed["messages"][1]["content"][0]["text"], "x");
        assert!(build_request_bytes(
            "m",
            8,
            0.1,
            "x",
            &images,
            "low",
            false,
            false,
            false,
            None,
            Some("{"),
            None,
        )
        .is_err());
    }

    #[test]
    fn body_has_system_text_images_and_controls() {
        let body = build_request_body(
            "m",
            16,
            0.5,
            "describe",
            &[image("a.png", 3)],
            "high",
            false,
            true,
            true,
            Some(4),
            Some(r#"{"top_p":0.9}"#),
            None,
        )
        .expect("body");
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages[0]["role"], "system");
        let content = messages[1]["content"].as_array().expect("content");
        assert_eq!(content[0]["text"], "describe");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
        assert_eq!(content[1]["image_url"]["detail"], "high");
        assert_eq!(body["max_tokens"], 16);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["ignore_eos"], true);
        assert_eq!(body["min_tokens"], 4);
        assert_eq!(body["top_p"], 0.9);
    }

    #[test]
    fn empty_system_prompt_is_omitted_and_server_side_download_keeps_url() {
        let body = build_request_body(
            "m",
            8,
            0.1,
            "",
            &[image("https://x/a.png", 3)],
            "low",
            true,
            false,
            false,
            None,
            None,
            Some(""),
        )
        .expect("body");
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 1);
        let content = messages[0]["content"].as_array().expect("content");
        assert_eq!(content.len(), 1, "no empty text part");
        assert_eq!(content[0]["image_url"]["url"], "https://x/a.png");
        assert!(body.get("stream_options").is_none());
        assert!(build_request_body(
            "m",
            8,
            0.1,
            "x",
            &[],
            "low",
            false,
            false,
            false,
            None,
            Some("{"),
            None
        )
        .is_err());
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        assert!(ImageCache::new(0).is_err());
        let mut cache = ImageCache::new(2).expect("cache");
        cache.put("a".into(), image("a", 1));
        cache.put("b".into(), image("b", 2));
        assert!(cache.get("a").is_some(), "touch a");
        cache.put("c".into(), image("c", 3));
        assert!(cache.get("b").is_none(), "b was least recently used");
        assert_eq!(cache.get("a").map(|i| i.size_bytes), Some(1));
        assert_eq!(cache.get("c").map(|i| i.size_bytes), Some(3));
        cache.put("a".into(), image("a", 9));
        assert_eq!(cache.get("a").map(|i| i.size_bytes), Some(9));
    }

    #[test]
    fn format_image_content_defaults_to_base64() {
        let part = format_image_content(&image("a.png", 3), &ImageContentOptions::default());
        assert_eq!(part["type"], "image_url");
        assert_eq!(part["image_url"]["detail"], "low");
        assert_eq!(part["image_url"]["url"], "data:image/png;base64,QUJD");
    }
}
