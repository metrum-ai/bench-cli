// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared JSONL prompt input loaders for metrum-ai-bench-cli-llm and metrum-ai-bench-cli-vlm.
//! One JSON object per line; fail fast with file/line errors.
//! Rejects .csv with a migration hint.
//! Metrum AI Bench LLM supports local paths and HTTP(S) URLs for the prompts source.
//! `read_utf8_from_path_or_url` is shared with metrum-ai-bench-cli-asr JSONL manifests and optional ground-truth files.

use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::thread;

/// Reject CSV and require JSONL; return a clear migration message.
fn reject_csv(path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let lower = path.to_lowercase();
    if lower.ends_with(".csv") {
        return Err(format!(
            "Prompt input must be JSONL, not CSV. File '{}' has a .csv extension. \
             Migrate to JSONL: one JSON object per line with a \"prompt\" field (metrum-ai-bench-cli-llm) \
             or \"prompt\" and \"image_urls\" (metrum-ai-bench-cli-vlm). Example: {{\"prompt\":\"Your prompt here\"}}",
            path
        )
        .into());
    }
    Ok(())
}

/// Returns true if the path is an HTTP or HTTPS URL (for fetching prompts remotely).
pub fn is_http_url(path: &str) -> bool {
    let path = path.trim();
    path.starts_with("http://") || path.starts_with("https://")
}

/// Read full UTF-8 text from a local file path or HTTP(S) URL (blocking GET).
/// Used for metrum-ai-bench-cli-llm/metrum-ai-bench-cli-vlm-style JSONL sources, metrum-ai-bench-cli-asr input manifests, ground-truth JSONL, etc.
pub fn read_utf8_from_path_or_url(source: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let source = source.trim();
    if is_http_url(source) {
        let source_owned = source.to_string();
        let join_source = source_owned.clone();
        match thread::spawn(move || -> Result<String, Box<dyn Error + Send + Sync>> {
            let resp = reqwest::blocking::get(&source_owned)
                .map_err(|e| format!("Failed to fetch URL '{}': {}", source_owned, e))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(
                    format!("URL '{}' returned HTTP status {}", source_owned, status).into(),
                );
            }
            resp.text().map_err(|e| {
                format!(
                    "Failed to read response body from '{}': {}",
                    source_owned, e
                )
                .into()
            })
        })
        .join()
        {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!("Fetch thread panicked for '{}'", join_source).into()),
        }
    } else {
        std::fs::read_to_string(source)
            .map_err(|e| format!("Failed to read file '{}': {}", source, e).into())
    }
}

/// Parse JSONL content (one JSON object per line with "prompt" field) into a list of prompt strings.
/// `path` is used only for error messages (file path or URL).
fn parse_metrum_ai_bench_llm_jsonl_content(
    content: &str,
    path: &str,
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
    let mut prompts = Vec::new();
    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            format!(
                "Invalid JSON at line {} in '{}': {}. Line content: {}",
                line_num,
                path,
                e,
                truncate_for_error(line)
            )
        })?;
        let obj = value.as_object().ok_or_else(|| {
            format!(
                "Expected JSON object at line {} in '{}', got other type",
                line_num, path
            )
        })?;
        let prompt = obj.get("prompt").and_then(|v| v.as_str()).ok_or_else(|| {
            format!(
                "Missing required field \"prompt\" at line {} in '{}'",
                line_num, path
            )
        })?;
        prompts.push(prompt.to_string());
    }
    if prompts.is_empty() {
        return Err(format!(
            "No prompts found in '{}' (empty file or no valid lines)",
            path
        )
        .into());
    }
    Ok(prompts)
}

/// Load metrum-ai-bench-cli-llm prompts from a JSONL file or HTTP(S) URL.
/// Path may be a local file path or an http:// / https:// URL; content must be JSONL.
/// Minimum contract: one object per line with "prompt" (string).
/// Preserves embedded newlines in prompt text. Fails fast with file/line or URL/status errors.
pub fn load_metrum_ai_bench_llm_prompts(
    path: &str,
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
    reject_csv(path)?;
    let content = read_utf8_from_path_or_url(path)?;
    parse_metrum_ai_bench_llm_jsonl_content(&content, path)
}

/// Normalize an image reference for metrum-ai-bench-cli-vlm: strip file:// to a path; leave http(s) and plain paths as-is.
/// file:///path/to/file -> /path/to/file (Unix); file:///C:/foo -> /C:/foo (Windows).
pub fn normalize_image_ref(s: &str) -> String {
    let s = s.trim();
    if let Some(stripped) = s.strip_prefix("file://") {
        stripped.trim().to_string()
    } else {
        s.to_string()
    }
}

/// Load metrum-ai-bench-cli-vlm records from a JSONL file.
/// Minimum contract: one object per line with "prompt" (string) and "image_urls" (array of strings).
/// Accepts optional "image_url" (string) for single-image rows and normalizes to image_urls internally.
/// Image entries support HTTP(S) URLs, file:// URIs, and plain local paths; file:// is normalized to a path.
pub type VlmInputRecord = (String, Vec<String>);

pub fn load_metrum_ai_bench_vlm_records(
    path: &str,
) -> Result<Vec<VlmInputRecord>, Box<dyn Error + Send + Sync>> {
    reject_csv(path)?;

    let file =
        File::open(path).map_err(|e| format!("Failed to open prompts file '{}': {}", path, e))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line_num = line_num + 1;
        let line =
            line.map_err(|e| format!("Failed to read line {} in '{}': {}", line_num, path, e))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            format!(
                "Invalid JSON at line {} in '{}': {}. Line content: {}",
                line_num,
                path,
                e,
                truncate_for_error(line)
            )
        })?;
        let obj = value.as_object().ok_or_else(|| {
            format!(
                "Expected JSON object at line {} in '{}', got other type",
                line_num, path
            )
        })?;
        let prompt = obj.get("prompt").and_then(|v| v.as_str()).ok_or_else(|| {
            format!(
                "Missing required field \"prompt\" at line {} in '{}'",
                line_num, path
            )
        })?;

        let image_urls: Vec<String> = if let Some(arr) =
            obj.get("image_urls").and_then(|v| v.as_array())
        {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(normalize_image_ref)
                .collect()
        } else if let Some(s) = obj.get("image_url").and_then(|v| v.as_str()) {
            vec![normalize_image_ref(s)]
        } else {
            return Err(format!(
                "Missing required field \"image_urls\" (or single \"image_url\") at line {} in '{}'",
                line_num, path
            )
            .into());
        };

        records.push((prompt.to_string(), image_urls));
    }

    if records.is_empty() {
        return Err(format!(
            "No valid records found in '{}' (empty file or no valid lines)",
            path
        )
        .into());
    }
    Ok(records)
}

fn truncate_for_error(s: &str) -> String {
    const MAX: usize = 120;
    if s.len() <= MAX {
        s.to_string()
    } else {
        format!("{}...", &s[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn test_reject_csv_metrum_ai_bench_llm() {
        let err = load_metrum_ai_bench_llm_prompts("/tmp/prompts.csv").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("JSONL"), "{}", msg);
        assert!(msg.contains(".csv"), "{}", msg);
    }

    #[test]
    fn test_reject_csv_metrum_ai_bench_vlm() {
        let err = load_metrum_ai_bench_vlm_records("/tmp/prompts.csv").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("JSONL"), "{}", msg);
    }

    #[test]
    fn test_load_metrum_ai_bench_llm_prompts_valid() {
        let tmp = std::env::temp_dir().join("metrum_ai_bench_llm_prompts_test.jsonl");
        let content = r#"{"prompt":"Hello"}
{"prompt":"World with, comma"}
{"prompt":"Quote \"inside\""}
{"prompt":"New\nline"}"#;
        std::fs::write(&tmp, content).unwrap();
        let prompts = load_metrum_ai_bench_llm_prompts(tmp.to_str().unwrap()).unwrap();
        assert_eq!(prompts.len(), 4);
        assert_eq!(prompts[0], "Hello");
        assert_eq!(prompts[1], "World with, comma");
        assert_eq!(prompts[2], "Quote \"inside\"");
        assert_eq!(prompts[3], "New\nline");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_metrum_ai_bench_llm_prompts_missing_prompt() {
        let tmp = std::env::temp_dir().join("metrum_ai_bench_llm_missing.jsonl");
        std::fs::write(&tmp, r#"{"other":"field"}"#).unwrap();
        let err = load_metrum_ai_bench_llm_prompts(tmp.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("prompt"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_metrum_ai_bench_llm_prompts_malformed_json() {
        let tmp = std::env::temp_dir().join("metrum_ai_bench_llm_bad.jsonl");
        std::fs::write(&tmp, r#"not json"#).unwrap();
        let err = load_metrum_ai_bench_llm_prompts(tmp.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("line 1"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_normalize_image_ref() {
        assert_eq!(
            normalize_image_ref("https://example.com/a.jpg"),
            "https://example.com/a.jpg"
        );
        assert_eq!(
            normalize_image_ref("file:///path/to/img.jpg"),
            "/path/to/img.jpg"
        );
        assert_eq!(normalize_image_ref("  file:///C:/foo/bar  "), "/C:/foo/bar");
        assert_eq!(normalize_image_ref("/local/path"), "/local/path");
    }

    #[test]
    fn test_load_metrum_ai_bench_vlm_records_valid() {
        let tmp = std::env::temp_dir().join("metrum_ai_bench_vlm_test.jsonl");
        let content = r#"{"prompt":"What is this?","image_urls":["https://example.com/a.jpg"]}
{"prompt":"Describe","image_url":"file:///path/to/local.jpg"}
{"prompt":"Multi","image_urls":["https://a.com/1.jpg","https://b.com/2.jpg"]}"#;
        std::fs::write(&tmp, content).unwrap();
        let records = load_metrum_ai_bench_vlm_records(tmp.to_str().unwrap()).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].0, "What is this?");
        assert_eq!(records[0].1, vec!["https://example.com/a.jpg"]);
        assert_eq!(records[1].1, vec!["/path/to/local.jpg"]);
        assert_eq!(records[2].1.len(), 2);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_metrum_ai_bench_vlm_records_missing_image_urls() {
        let tmp = std::env::temp_dir().join("metrum_ai_bench_vlm_no_urls.jsonl");
        std::fs::write(&tmp, r#"{"prompt":"No images"}"#).unwrap();
        let err = load_metrum_ai_bench_vlm_records(tmp.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("image_urls"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_reject_csv_url() {
        let err = load_metrum_ai_bench_llm_prompts("https://example.com/prompts.csv").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("JSONL"), "{}", msg);
        assert!(msg.contains(".csv"), "{}", msg);
    }

    #[test]
    fn test_is_http_url() {
        assert!(is_http_url("https://example.com/prompts.jsonl"));
        assert!(is_http_url("http://localhost:8000/prompts.jsonl"));
        assert!(!is_http_url("/local/path/prompts.jsonl"));
        assert!(!is_http_url("prompts.jsonl"));
    }

    #[test]
    fn test_read_utf8_from_path_or_url_file() {
        let tmp = std::env::temp_dir().join("metrum_ai_bench_read_utf8_file_test.txt");
        std::fs::write(&tmp, "hello utf8").unwrap();
        let s = read_utf8_from_path_or_url(tmp.to_str().unwrap()).unwrap();
        assert_eq!(s, "hello utf8");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_read_utf8_from_path_or_url_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let body = "{\"id\":\"a\",\"transcript\":\"hi\"}\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/jsonl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        let url = format!("http://{}/manifest.jsonl", addr);
        let text = read_utf8_from_path_or_url(&url).unwrap();
        server.join().unwrap();
        assert_eq!(text, "{\"id\":\"a\",\"transcript\":\"hi\"}\n");
    }

    #[test]
    fn test_load_metrum_ai_bench_llm_prompts_url_inside_tokio_runtime() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let body = "{\"prompt\":\"Hello from URL\"}\n{\"prompt\":\"Second prompt\"}\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/jsonl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = format!("http://{}/prompts.jsonl", addr);
        let prompts = runtime
            .block_on(async { load_metrum_ai_bench_llm_prompts(&url) })
            .unwrap();

        server.join().unwrap();

        assert_eq!(prompts, vec!["Hello from URL", "Second prompt"]);
    }
}
