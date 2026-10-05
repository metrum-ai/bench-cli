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
use std::time::Duration;

/// Loaded image bytes (base64) plus the metadata recorded per request.
#[derive(Clone)]
pub struct ImageData {
    pub base64_data: String,
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
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).expect("index from position");
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
            let decoded = crate::prompt_inputs::decode_data_url(path)?;
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
        let (source_width, source_height) =
            image::ImageReader::new(std::io::Cursor::new(&image_data))
                .with_guessed_format()
                .map_err(|e| format!("Failed to read image header from '{}': {}", label, e))?
                .into_dimensions()
                .map_err(|e| format!("Failed to read image size from '{}': {}", label, e))?;

        let oversized =
            max_dimension.is_some_and(|max_dim| source_width > max_dim || source_height > max_dim);

        let (encoded, width, height) = if oversized || reencode_jpeg {
            let mut img = image::load_from_memory(&image_data)
                .map_err(|e| format!("Failed to decode image from '{}': {}", label, e))?;
            if oversized {
                let max_dim = max_dimension.expect("oversized implies a limit");
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

    let body_str = serde_json::to_string_pretty(&body).unwrap_or_default();
    if body_str.contains("base64") {
        debug!("Built request body: (redacted: contains image data)");
    } else {
        debug!("Built request body: {}", body_str);
    }
    Ok(body)
}

