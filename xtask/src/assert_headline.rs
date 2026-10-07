// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use flate2::read::ZlibDecoder;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn run(modality: &str, log: &Path, artifact_dir: Option<&Path>) -> Result<()> {
    match modality {
        "llm" | "vlm" | "asr" | "imagegen" => {}
        _ => {
            eprintln!(
                "Usage: cargo xtask assert-headline <llm|vlm|asr|imagegen> <data_log.jsonl> [--artifact-dir DIR]"
            );
            std::process::exit(2);
        }
    }
    if !log.is_file() {
        eprintln!(
            "assert_headline: FAIL {modality}: no data log at {}",
            log.display()
        );
        std::process::exit(1);
    }
    let min_ratio: f64 = std::env::var("MIN_SUCCESS_RATIO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    match evaluate(modality, log, artifact_dir, min_ratio) {
        Ok(()) => Ok(()),
        Err(e) => {
            // evaluate prints the report; exit 1
            let _ = e;
            std::process::exit(1);
        }
    }
}

pub fn evaluate(
    modality: &str,
    log: &Path,
    artifact_dir: Option<&Path>,
    min_ratio: f64,
) -> Result<()> {
    let text = std::fs::read_to_string(log).with_context(|| format!("read {}", log.display()))?;
    let mut failures: Vec<String> = Vec::new();
    let mut rows: Vec<Value> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) => rows.push(v),
            Err(e) => failures.push(format!("line {} is not JSON: {e}", n + 1)),
        }
    }

    let schema = |r: &Value| -> String {
        r.get("schema_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let summary = rows
        .iter()
        .rev()
        .find(|r| schema(r).contains("summary.v"))
        .cloned();
    let measured: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            schema(r).contains("request.v")
                && r.get("phase").and_then(|p| p.as_str()).unwrap_or("measure") == "measure"
        })
        .collect();
    let ok: Vec<&Value> = measured
        .iter()
        .copied()
        .filter(|r| r.get("error").map(|e| e.is_null()).unwrap_or(true))
        .collect();

    if summary.is_none() {
        failures.push("no summary.v3 line (run crashed or was killed)".into());
    } else if let Some(ref summary) = summary {
        let common = summary
            .pointer("/config/common")
            .cloned()
            .unwrap_or(Value::Null);
        if common.get("require_sut") != Some(&Value::Bool(true)) {
            failures.push("run did not set --require-sut".into());
        }
        if summary.get("sut").map(|s| s.is_null()).unwrap_or(true) {
            failures.push("summary has no sut block".into());
        }
        let attempted = summary
            .get("attempted")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let successes = summary
            .get("successes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if successes == 0 {
            failures.push(format!(
                "0 successes out of {attempted} attempted (errors_by_type={:?})",
                summary.get("errors_by_type")
            ));
        } else if attempted > 0 && (successes as f64) / (attempted as f64) < min_ratio {
            failures.push(format!(
                "success ratio {successes}/{attempted} below MIN_SUCCESS_RATIO={min_ratio}"
            ));
        }
    }

    fn metric_num(r: &Value, key: &str) -> Option<f64> {
        r.get("modality_metrics")
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
    }
    fn metric_present(r: &Value, key: &str) -> bool {
        r.get("modality_metrics")
            .and_then(|m| m.get(key))
            .map(|v| !v.is_null())
            .unwrap_or(false)
    }

    let mut extra = String::new();
    if modality == "asr" && !ok.is_empty() {
        let missing: Vec<_> = ok
            .iter()
            .filter(|r| !metric_present(r, "wer") || !metric_present(r, "cer"))
            .map(|r| r.get("seq").cloned().unwrap_or(Value::Null))
            .collect();
        if !missing.is_empty() {
            let preview: Vec<_> = missing.iter().take(5).collect();
            failures.push(format!(
                "{} successful ASR records lack wer/cer (pass --ground-truth); seq={preview:?}",
                missing.len()
            ));
        } else {
            let wer: f64 = ok
                .iter()
                .map(|r| metric_num(r, "wer").unwrap_or(0.0))
                .sum::<f64>()
                / ok.len() as f64;
            let cer: f64 = ok
                .iter()
                .map(|r| metric_num(r, "cer").unwrap_or(0.0))
                .sum::<f64>()
                / ok.len() as f64;
            extra = format!(" mean_wer={wer:.4} mean_cer={cer:.4} n={}", ok.len());
        }
    } else if modality == "vlm" && !ok.is_empty() {
        let zero: Vec<_> = ok
            .iter()
            .filter(|r| metric_num(r, "image_count").unwrap_or(0.0) == 0.0)
            .map(|r| r.get("seq").cloned().unwrap_or(Value::Null))
            .collect();
        if !zero.is_empty() {
            let preview: Vec<_> = zero.iter().take(5).collect();
            failures.push(format!(
                "{} successful VLM records sent no image (image_count 0); seq={preview:?}",
                zero.len()
            ));
        }
    } else if modality == "imagegen" {
        let returned: u64 = ok
            .iter()
            .map(|r| metric_num(r, "images_returned").unwrap_or(0.0) as u64)
            .sum();
        if returned == 0 {
            failures.push("no successful imagegen record returned images".into());
        }
        if let Some(dir) = artifact_dir {
            let mut files = Vec::new();
            collect_images(dir, &mut files)?;
            let good: Vec<_> = files.iter().filter(|p| decodes(p)).collect();
            if good.is_empty() {
                failures.push(format!(
                    "no decodable PNG/JPEG artifacts in {} ({} image files)",
                    dir.display(),
                    files.len()
                ));
            }
            extra = format!(" decoded_artifacts={}/{}", good.len(), files.len());
        }
    }

    let status = if failures.is_empty() { "PASS" } else { "FAIL" };
    println!(
        "assert_headline: {status} {modality} {} measured_ok={}/{}{extra}",
        log.display(),
        ok.len(),
        measured.len()
    );
    for f in &failures {
        println!("  - {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("assert_headline failed")
    }
}

fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_images(&path, out)?;
        } else if entry.file_type()?.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn decodes(path: &Path) -> bool {
    let Ok(b) = std::fs::read(path) else {
        return false;
    };
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut pos = 8usize;
        let mut idat = Vec::new();
        let mut w = 0u64;
        while pos + 8 <= b.len() {
            let size = u32::from_be_bytes([b[pos], b[pos + 1], b[pos + 2], b[pos + 3]]) as usize;
            let kind = &b[pos + 4..pos + 8];
            if pos + 8 + size > b.len() {
                break;
            }
            let data = &b[pos + 8..pos + 8 + size];
            if kind == b"IHDR" && data.len() >= 8 {
                let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
                let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as u64;
                w = width.saturating_mul(height);
            } else if kind == b"IDAT" {
                idat.extend_from_slice(data);
            }
            pos += 12 + size;
        }
        let mut dec = ZlibDecoder::new(&idat[..]);
        let mut buf = Vec::new();
        return w > 0 && dec.read_to_end(&mut buf).is_ok() && !buf.is_empty();
    }
    // JPEG: SOI .. EOI after stripping trailing NULs
    let mut end = b.len();
    while end > 0 && b[end - 1] == 0 {
        end -= 1;
    }
    b.starts_with(b"\xff\xd8") && end >= 2 && b[end - 2] == 0xff && b[end - 1] == 0xd9
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn req(seq: u64, error: &str, metrics: &str) -> String {
        format!(
            r#"{{"schema_version":"metrum-ai-bench-cli.request.v3","seq":{seq},"phase":"measure","error":{error},"modality_metrics":{metrics}}}"#
        )
    }
    fn summary(attempted: u64, successes: u64, require_sut: bool) -> String {
        format!(
            r#"{{"schema_version":"metrum-ai-bench-cli.summary.v3","attempted":{attempted},"successes":{successes},"errors_by_type":{{}},"sut":{{"name":"x"}},"config":{{"common":{{"require_sut":{require_sut}}}}}}}"#
        )
    }

    fn write_log(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn assert_headline_pass_and_fail_cases() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path();

        let asr_ok = write_log(
            work,
            "asr-ok.jsonl",
            &format!(
                "{}\n{}\n{}\n",
                req(0, "null", r#"{"wer":0.1,"cer":0.05}"#),
                req(1, "null", r#"{"wer":0.0,"cer":0.0}"#),
                summary(2, 2, true)
            ),
        );
        assert!(evaluate("asr", &asr_ok, None, 1.0).is_ok());

        let asr_nower = write_log(
            work,
            "asr-nower.jsonl",
            &format!(
                "{}\n{}\n",
                req(0, "null", r#"{"rtfx_client":3.0}"#),
                summary(1, 1, true)
            ),
        );
        assert!(evaluate("asr", &asr_nower, None, 1.0).is_err());

        let zero = write_log(
            work,
            "zero.jsonl",
            &format!(
                "{}\n{}\n",
                req(0, r#"{"kind":"http_status","status":400}"#, "{}"),
                summary(1, 0, true)
            ),
        );
        assert!(evaluate("llm", &zero, None, 1.0).is_err());

        let half = write_log(
            work,
            "half.jsonl",
            &format!(
                "{}\n{}\n{}\n",
                req(0, "null", "{}"),
                req(1, r#"{"kind":"timeout"}"#, "{}"),
                summary(2, 1, true)
            ),
        );
        assert!(evaluate("llm", &half, None, 1.0).is_err());
        assert!(evaluate("llm", &half, None, 0.5).is_ok());

        let nosut = write_log(
            work,
            "nosut.jsonl",
            &format!("{}\n{}\n", req(0, "null", "{}"), summary(1, 1, false)),
        );
        assert!(evaluate("llm", &nosut, None, 1.0).is_err());

        let vlm = write_log(
            work,
            "vlm-noimg.jsonl",
            &format!(
                "{}\n{}\n",
                req(0, "null", r#"{"image_count":0}"#),
                summary(1, 1, true)
            ),
        );
        assert!(evaluate("vlm", &vlm, None, 1.0).is_err());

        let img_none = write_log(
            work,
            "img-none.jsonl",
            &format!(
                "{}\n{}\n",
                req(0, "null", r#"{"images_returned":0}"#),
                summary(1, 1, true)
            ),
        );
        assert!(evaluate("imagegen", &img_none, None, 1.0).is_err());

        let img_ok = write_log(
            work,
            "img-ok.jsonl",
            &format!(
                "{}\n{}\n",
                req(0, "null", r#"{"images_returned":1}"#),
                summary(1, 1, true)
            ),
        );
        let art_bad = work.join("art-bad");
        std::fs::create_dir_all(&art_bad).unwrap();
        std::fs::write(art_bad.join("000001-0.png"), b"not a png").unwrap();
        assert!(evaluate("imagegen", &img_ok, Some(&art_bad), 1.0).is_err());

        // Prefer repo fixture if present; else skip good-artifact pass.
        let repo_png =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-data/vlm/shapes-512.png");
        if repo_png.is_file() {
            let art_good = work.join("art-good");
            std::fs::create_dir_all(&art_good).unwrap();
            std::fs::copy(&repo_png, art_good.join("000001-0.png")).unwrap();
            assert!(evaluate("imagegen", &img_ok, Some(&art_good), 1.0).is_ok());
        }

        let mut truncated = std::fs::File::create(work.join("truncated.jsonl")).unwrap();
        truncated
            .write_all(&summary(0, 0, true).as_bytes()[..20])
            .unwrap();
        assert!(evaluate("llm", &work.join("truncated.jsonl"), None, 1.0).is_err());
    }
}
