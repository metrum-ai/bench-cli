// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Streaming Prometheus text / OpenMetrics exposition parser.
//!
//! Hot path avoids regex. `include` filtering uses a precompiled RegexSet on
//! the metric name only, before label parsing.

use super::row::MetricType;
use regex::RegexSet;
use std::collections::BTreeMap;

/// One parsed sample from an exposition scrape.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSample {
    pub metric: String,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
    pub mtype: MetricType,
}

/// Result of parsing an exposition body.
#[derive(Debug, Default)]
pub struct ParseResult {
    pub samples: Vec<ParsedSample>,
    pub skipped_lines: u64,
    pub malformed_lines: u64,
}

/// Parse Prometheus text exposition (0.0.4) and trivially compatible OpenMetrics.
///
/// `include` is applied to the metric name only. Metrics that do not match are
/// skipped before labels are parsed.
pub fn parse_exposition(text: &str, include: &RegexSet) -> ParseResult {
    let mut result = ParseResult::default();
    let mut type_map: BTreeMap<String, MetricType> = BTreeMap::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "# EOF" {
            break;
        }
        if line.starts_with('#') {
            parse_meta(line, &mut type_map);
            continue;
        }
        match parse_sample_line(line, &type_map, include) {
            SampleOutcome::Keep(sample) => result.samples.push(sample),
            SampleOutcome::Skipped => result.skipped_lines += 1,
            SampleOutcome::Malformed => result.malformed_lines += 1,
        }
    }
    result
}

fn parse_meta(line: &str, type_map: &mut BTreeMap<String, MetricType>) {
    // # TYPE name type
    // # HELP name text...
    let rest = line.trim_start_matches('#').trim_start();
    if let Some(after) = rest.strip_prefix("TYPE ") {
        let mut parts = after.split_whitespace();
        let Some(name) = parts.next() else {
            return;
        };
        let Some(ty) = parts.next() else {
            return;
        };
        type_map.insert(name.to_string(), map_type(ty));
    }
    // HELP is recorded only for future unit hints; we do not store it yet.
}

fn map_type(ty: &str) -> MetricType {
    match ty.to_ascii_lowercase().as_str() {
        "counter" => MetricType::Counter,
        "gauge" => MetricType::Gauge,
        "histogram" => MetricType::HistogramBucket,
        "summary" => MetricType::Summary,
        "unknown" | "untyped" | "info" | "stateset" => MetricType::Unknown,
        _ => MetricType::Unknown,
    }
}

enum SampleOutcome {
    Keep(ParsedSample),
    Skipped,
    Malformed,
}

fn parse_sample_line(
    line: &str,
    type_map: &BTreeMap<String, MetricType>,
    include: &RegexSet,
) -> SampleOutcome {
    // Drop exemplars: everything after " # " that is not part of labels.
    let line = strip_exemplar(line);

    let (name, labels, value_str) = match split_sample(line) {
        Some(parts) => parts,
        None => return SampleOutcome::Malformed,
    };

    if include.is_empty() || !include.is_match(name) {
        return SampleOutcome::Skipped;
    }

    let value = match parse_float(value_str) {
        Some(v) => v,
        None => return SampleOutcome::Malformed,
    };

    let labels = match labels {
        Some(raw) => match parse_labels(raw) {
            Some(map) => map,
            None => return SampleOutcome::Malformed,
        },
        None => BTreeMap::new(),
    };

    let mtype = infer_type(name, type_map);
    SampleOutcome::Keep(ParsedSample {
        metric: name.to_string(),
        labels,
        value,
        mtype,
    })
}

fn strip_exemplar(line: &str) -> &str {
    // Exemplars appear after the value: `metric{...} 1 # {id="x"} 0.5`
    // Find " # " after the value region. Simplest: if we see " # {" trim there.
    if let Some(idx) = line.find(" # ") {
        // Only strip when it looks like an exemplar, not a weird metric name.
        let after = &line[idx + 3..];
        if after.starts_with('{') || after.starts_with('+') || after.starts_with('-') {
            return &line[..idx];
        }
    }
    line
}

fn split_sample(line: &str) -> Option<(&str, Option<&str>, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_metric_name_byte(bytes[i]) {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let name = &line[..i];
    let rest = line[i..].trim_start();
    if rest.starts_with('{') {
        let end = find_label_close(rest)?;
        let labels = &rest[1..end];
        let after = rest[end + 1..].trim_start();
        let value = first_token(after)?;
        Some((name, Some(labels), value))
    } else {
        let value = first_token(rest)?;
        Some((name, None, value))
    }
}

fn is_metric_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b':'
}

fn find_label_close(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut in_quote = false;
    let mut escape = false;
    for (idx, &b) in bytes.iter().enumerate().skip(1) {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_quote => escape = true,
            b'"' => in_quote = !in_quote,
            b'}' if !in_quote => return Some(idx),
            _ => {}
        }
    }
    None
}

fn first_token(s: &str) -> Option<&str> {
    let tok = s.split_whitespace().next()?;
    // OpenMetrics may append a timestamp after the value; take first token only.
    Some(tok)
}

fn parse_float(s: &str) -> Option<f64> {
    match s {
        "NaN" | "+NaN" | "-NaN" => Some(f64::NAN),
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        other => other.parse().ok(),
    }
}

fn parse_labels(raw: &str) -> Option<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b',' || bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i == key_start || i >= bytes.len() || bytes[i] != b'=' {
            return None;
        }
        let key = &raw[key_start..i];
        i += 1; // =
        if i >= bytes.len() || bytes[i] != b'"' {
            return None;
        }
        i += 1; // opening quote
        let mut value = String::new();
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' {
                i += 1;
                if i >= bytes.len() {
                    return None;
                }
                match bytes[i] {
                    b'n' => value.push('\n'),
                    b'\\' => value.push('\\'),
                    b'"' => value.push('"'),
                    other => value.push(other as char),
                }
                i += 1;
                continue;
            }
            if b == b'"' {
                i += 1;
                break;
            }
            value.push(b as char);
            i += 1;
        }
        map.insert(key.to_string(), value);
    }
    Some(map)
}

fn infer_type(name: &str, type_map: &BTreeMap<String, MetricType>) -> MetricType {
    if let Some(ty) = type_map.get(name) {
        return *ty;
    }
    // Histogram/summary family: TYPE is on the base name.
    if let Some(base) = name.strip_suffix("_bucket") {
        if type_map.get(base) == Some(&MetricType::HistogramBucket) {
            return MetricType::HistogramBucket;
        }
    }
    if let Some(base) = name
        .strip_suffix("_sum")
        .or_else(|| name.strip_suffix("_count"))
    {
        if matches!(
            type_map.get(base),
            Some(MetricType::HistogramBucket | MetricType::Summary)
        ) {
            return *type_map.get(base).unwrap();
        }
    }
    if name.ends_with("_total") {
        return MetricType::Counter;
    }
    if name.ends_with("_bucket") {
        return MetricType::HistogramBucket;
    }
    MetricType::Unknown
}

/// Engine allowlist used when desugaring `--metrics-url`.
pub fn engine_include_patterns() -> Vec<String> {
    vec![
        r"^vllm:(gpu_cache_usage_perc|kv_cache_usage_perc|num_requests_(running|waiting)|num_preemptions_total|prompt_tokens_total|generation_tokens_total)$".into(),
        r"^sglang:(token_usage|num_running_reqs|num_queue_reqs|cache_hit_rate|num_preemptions_total)$".into(),
        r"^trtllm_(kv_cache_utilization|request_preemptions|request_metrics_(active|queued))$".into(),
        r"^nv_trt_llm_(kv_cache_block_metrics|request_metrics)$".into(),
        r"^nv_inference_queue_duration_us$".into(),
        r"^llamacpp:(kv_cache_usage_ratio|requests_processing|tokens_predicted_total)$".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(patterns: &[&str]) -> RegexSet {
        RegexSet::new(patterns).expect("regex")
    }

    #[test]
    fn parses_gauge_with_labels_and_type() {
        let text = "\
# HELP all_smi_gpu_power_consumption_watts GPU power
# TYPE all_smi_gpu_power_consumption_watts gauge
all_smi_gpu_power_consumption_watts{gpu=\"0\",uuid=\"GPU-abc\"} 250.5
";
        let include = set(&[r"^all_smi_gpu_"]);
        let result = parse_exposition(text, &include);
        assert_eq!(result.samples.len(), 1);
        assert_eq!(
            result.samples[0].metric,
            "all_smi_gpu_power_consumption_watts"
        );
        assert_eq!(result.samples[0].value, 250.5);
        assert_eq!(result.samples[0].mtype, MetricType::Gauge);
        assert_eq!(
            result.samples[0].labels.get("gpu").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn skips_non_matching_before_labels() {
        let text = "huge_metric{a=\"1\",b=\"2\",c=\"3\"} 1\nkeep_me 2\n";
        let include = set(&[r"^keep_me$"]);
        let result = parse_exposition(text, &include);
        assert_eq!(result.samples.len(), 1);
        assert_eq!(result.samples[0].metric, "keep_me");
        assert!(result.skipped_lines >= 1);
    }

    #[test]
    fn handles_nan_inf_and_escaped_labels() {
        let text = r#"m{path="a\"b\nc"} NaN
m2{} +Inf
m3{} -Inf
"#;
        let include = set(&[r"^m"]);
        let result = parse_exposition(text, &include);
        assert_eq!(result.samples.len(), 3);
        assert!(result.samples[0].value.is_nan());
        assert!(
            result.samples[1].value.is_infinite() && result.samples[1].value.is_sign_positive()
        );
        assert!(
            result.samples[2].value.is_infinite() && result.samples[2].value.is_sign_negative()
        );
        assert_eq!(
            result.samples[0].labels.get("path").map(String::as_str),
            Some("a\"b\nc")
        );
    }

    #[test]
    fn drops_exemplars_and_openmetrics_eof() {
        let text = "\
# TYPE http_requests_total counter
http_requests_total{code=\"200\"} 1027 1395066363000 # {span_id=\"abc\"} 1.0
# EOF
http_requests_total{code=\"500\"} 3
";
        let include = set(&[r"^http_requests_total$"]);
        let result = parse_exposition(text, &include);
        assert_eq!(result.samples.len(), 1);
        assert_eq!(result.samples[0].value, 1027.0);
        assert_eq!(result.samples[0].mtype, MetricType::Counter);
    }

    #[test]
    fn counter_suffix_without_type_is_counter() {
        let text = "foo_total 9\n";
        let include = set(&[r"^foo_total$"]);
        let result = parse_exposition(text, &include);
        assert_eq!(result.samples[0].mtype, MetricType::Counter);
    }

    #[test]
    fn malformed_lines_are_counted() {
        let text = "ok{badlabels 1\nok 1\n";
        let include = set(&[r"^ok$"]);
        let result = parse_exposition(text, &include);
        assert_eq!(result.samples.len(), 1);
        assert!(result.malformed_lines >= 1);
    }

    #[test]
    fn parses_dcgm_style_fixture() {
        let mut body = String::from("# TYPE DCGM_FI_DEV_POWER_USAGE gauge\n");
        for i in 0..100 {
            body.push_str(&format!(
                "DCGM_FI_DEV_POWER_USAGE{{gpu=\"{i}\"}} {}\n",
                100.0 + i as f64
            ));
            body.push_str(&format!(
                "DCGM_FI_DEV_GPU_UTIL{{gpu=\"{i}\"}} {}\n",
                i as f64
            ));
            body.push_str(&format!("ignored_metric{{gpu=\"{i}\"}} 1\n"));
        }
        let include = set(&[r"^DCGM_FI_DEV_POWER_USAGE$", r"^DCGM_FI_DEV_GPU_UTIL$"]);
        let result = parse_exposition(&body, &include);
        assert_eq!(result.samples.len(), 200);
        assert!(result.skipped_lines >= 100);
    }
}
