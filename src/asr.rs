// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Whisper-compatible English normalization and ASR accuracy metrics, plus
//! the audio inputs and transcription request shared by
//! `metrum-ai-bench-cli-asr` and the strategic `--kind asr` sweep. Metrum AI.

use crate::prompt_inputs::read_utf8_from_path_or_url;
use log::info;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[clap(rename_all = "kebab-case")]
pub enum Normalizer {
    /// Whisper basic normalization plus English contraction and numeral folding.
    #[default]
    WhisperEnglish,
    /// Case, punctuation and bracketed-filler folding only.
    WhisperBasic,
    /// Compare raw strings.
    None,
}

impl std::fmt::Display for Normalizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Normalizer::WhisperEnglish => write!(f, "whisper-english"),
            Normalizer::WhisperBasic => write!(f, "whisper-basic"),
            Normalizer::None => write!(f, "none"),
        }
    }
}

pub fn normalize(text: &str, normalizer: Normalizer) -> String {
    match normalizer {
        Normalizer::None => text.to_string(),
        Normalizer::WhisperBasic => basic(text),
        Normalizer::WhisperEnglish => english(&basic(text)),
    }
}

fn basic(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_brackets = false;
    for character in text.to_lowercase().replace(['’', '`'], "'").chars() {
        match character {
            '[' | '(' | '<' => in_brackets = true,
            ']' | ')' | '>' => {
                in_brackets = false;
                result.push(' ');
            }
            _ if in_brackets => {}
            c if c.is_alphanumeric() || c == '\'' => result.push(c),
            _ => result.push(' '),
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn english(text: &str) -> String {
    text.split_whitespace()
        .flat_map(|word| match word {
            "can't" => vec!["can", "not"],
            "won't" => vec!["will", "not"],
            "i'm" => vec!["i", "am"],
            "it's" => vec!["it", "is"],
            "that's" => vec!["that", "is"],
            "there's" => vec!["there", "is"],
            "don't" => vec!["do", "not"],
            "doesn't" => vec!["does", "not"],
            "didn't" => vec!["did", "not"],
            "isn't" => vec!["is", "not"],
            "aren't" => vec!["are", "not"],
            "wasn't" => vec!["was", "not"],
            "weren't" => vec!["were", "not"],
            "zero" => vec!["0"],
            "one" => vec!["1"],
            "two" => vec!["2"],
            "three" => vec!["3"],
            "four" => vec!["4"],
            "five" => vec!["5"],
            "six" => vec!["6"],
            "seven" => vec!["7"],
            "eight" => vec!["8"],
            "nine" => vec!["9"],
            "ten" => vec!["10"],
            _ => vec![word],
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn word_error_rate(reference: &str, hypothesis: &str, normalizer: Normalizer) -> Option<f64> {
    let reference = normalize(reference, normalizer);
    let hypothesis = normalize(hypothesis, normalizer);
    let expected: Vec<_> = reference.split_whitespace().collect();
    let actual: Vec<_> = hypothesis.split_whitespace().collect();
    if expected.is_empty() {
        return actual.is_empty().then_some(0.0);
    }
    Some(edit_distance(&expected, &actual) as f64 / expected.len() as f64)
}

pub fn character_error_rate(
    reference: &str,
    hypothesis: &str,
    normalizer: Normalizer,
) -> Option<f64> {
    let reference: Vec<_> = normalize(reference, normalizer).chars().collect();
    let hypothesis: Vec<_> = normalize(hypothesis, normalizer).chars().collect();
    if reference.is_empty() {
        return hypothesis.is_empty().then_some(0.0);
    }
    Some(edit_distance(&reference, &hypothesis) as f64 / reference.len() as f64)
}

fn edit_distance<T: Eq>(expected: &[T], actual: &[T]) -> usize {
    let mut previous: Vec<usize> = (0..=actual.len()).collect();
    for (i, expected_item) in expected.iter().enumerate() {
        let mut current = vec![i + 1; actual.len() + 1];
        for (j, actual_item) in actual.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(expected_item != actual_item));
        }
        previous = current;
    }
    previous[actual.len()]
}

/// Real-time-factor multiplier (higher is better), using client wall duration.
pub fn rtfx(audio_seconds: f64, client_seconds: f64) -> Option<f64> {
    (audio_seconds.is_finite() && client_seconds.is_finite() && client_seconds > 0.0)
        .then_some(audio_seconds / client_seconds)
}

// Audio inputs and the transcription request.

/// One audio input row (`id`, `path` or `url`, `format`, optional `duration`).
#[derive(Clone, Debug)]
pub struct AudioSample {
    pub id: String,
    pub url: Option<String>, // URL is now optional if we have a direct path
    pub format: String,
    pub duration: Option<f64>,
    pub ground_truth: Option<String>,
    // Stores the local file path (either from direct path or after download)
    pub local_file_path: Option<String>,
}

/// Upload MIME type for an audio `format` (defaults to `audio/mpeg`).
pub fn mime_for_format(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "mp3" | "mpeg" => "audio/mpeg",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        "ogg" | "oga" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "flac" => "audio/flac",
        _ => "audio/mpeg",
    }
}

/// Collision-safe cache key from URL so different URLs never share the same file.
fn url_cache_key(url: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

/// Download `url` into `./audio/<hash>.<format>` once and return the path.
pub async fn download_audio_file(
    client: &Client,
    url: &str,
    format: &str,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let temp_dir = PathBuf::from("audio");
    if !temp_dir.exists() {
        fs::create_dir_all(&temp_dir)?;
    }
    // Use URL-based cache key to avoid collisions when different URLs have the same basename
    let file_name = format!("{:016x}.{}", url_cache_key(url), format);
    let file_path = temp_dir.join(&file_name);
    let file_path_str = file_path.to_string_lossy().into_owned();

    // Check if file already exists
    if file_path.exists() {
        let metadata = fs::metadata(&file_path)?;
        info!(
            "Using existing audio file at {} (size: {} bytes)",
            file_path.display(),
            metadata.len()
        );
        return Ok(file_path_str);
    }

    info!(
        "Downloading audio file from {} to {}",
        url,
        file_path.display()
    );

    // Download the file
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Failed to download file: HTTP status {}", response.status()).into());
    }

    // Save the file
    let content = response.bytes().await?;
    fs::write(&file_path, content)?;

    // Verify file was written successfully
    let metadata = fs::metadata(&file_path)?;
    info!(
        "Successfully downloaded audio file to {} (size: {} bytes)",
        file_path.display(),
        metadata.len()
    );

    Ok(file_path_str)
}

/// Load audio sample rows from a JSONL file or http(s) URL.
pub fn load_audio_samples(
    input_path: &str,
) -> Result<Vec<AudioSample>, Box<dyn Error + Send + Sync>> {
    let content = read_utf8_from_path_or_url(input_path)?;
    let mut samples = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let sample: Value = serde_json::from_str(line)?;

        let id = sample
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Missing id field in audio sample")?
            .to_string();

        // Check if we have a direct path first, then fall back to URL
        let path = sample
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let url = sample
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Ensure we have either a path or URL
        if path.is_none() && url.is_none() {
            return Err("Audio sample must have either 'path' or 'url' field".into());
        }

        let format = sample
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let duration = sample.get("duration").and_then(|v| v.as_f64());

        // If we have a direct path, use it as the local_file_path
        let local_file_path = path;

        samples.push(AudioSample {
            id,
            url,
            format,
            duration,
            ground_truth: None,
            local_file_path,
        });
    }

    Ok(samples)
}

/// Load `{"id", "transcript"}` reference rows keyed by sample id.
pub fn load_ground_truth(
    path: &str,
) -> Result<HashMap<String, String>, Box<dyn Error + Send + Sync>> {
    let content = read_utf8_from_path_or_url(path)?;
    let mut ground_truth = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let record: Value = serde_json::from_str(line)?;

        let id = record
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Missing id field in ground truth")?
            .to_string();

        let transcript = record
            .get("transcript")
            .and_then(|v| v.as_str())
            .ok_or("Missing transcript field in ground truth")?
            .to_string();

        ground_truth.insert(id, transcript);
    }

    Ok(ground_truth)
}

/// Build the OpenAI `/v1/audio/transcriptions` multipart form: `file`,
/// `model`, `response_format`, optional `language`, and word timestamps for
/// the JSON formats. `file_name` should be a basename (never a full path).
/// `file_content` is shared, not copied, so a preloaded sample can be sent
/// many times.
pub fn transcription_form(
    model: &str,
    response_format: &str,
    language: &str,
    file_name: String,
    format: &str,
    file_content: bytes::Bytes,
) -> Result<reqwest::multipart::Form, Box<dyn Error + Send + Sync>> {
    let length = file_content.len() as u64;
    let file_part =
        reqwest::multipart::Part::stream_with_length(reqwest::Body::from(file_content), length)
            .file_name(file_name)
            .mime_str(mime_for_format(format))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", response_format.to_string());
    if !language.is_empty() {
        form = form.text("language", language.to_string());
    }
    // Only add timestamp_granularities for json/verbose_json formats
    if response_format == "json" || response_format == "verbose_json" {
        form = form.text("timestamp_granularities[]", "word");
    }
    Ok(form)
}

/// A parsed transcription response.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub text: String,
    /// Server-reported `inference_time` (seconds) when present, finite and
    /// non-negative; `None` otherwise (callers fall back to the client clock).
    pub server_time: Option<f64>,
    /// The response `usage` object when the server sends one (JSON formats).
    pub usage: Option<Value>,
}

/// Parse a transcription body. Text, SRT and VTT bodies are the transcript
/// itself and carry no server time or usage.
pub fn parse_transcription(
    response_format: &str,
    body: &str,
) -> Result<Transcription, Box<dyn Error + Send + Sync>> {
    match response_format {
        "verbose_json" | "json" => {
            let json_resp: Value = serde_json::from_str(body)?;
            let text = json_resp
                .get("text")
                .or_else(|| json_resp.get("transcription"))
                .and_then(Value::as_str)
                .ok_or("No transcription text in response")?
                .to_string();
            let server_time = json_resp
                .get("inference_time")
                .and_then(Value::as_f64)
                .filter(|t| t.is_finite() && *t >= 0.0);
            Ok(Transcription {
                text,
                server_time,
                usage: json_resp.get("usage").cloned(),
            })
        }
        "text" | "srt" | "vtt" => Ok(Transcription {
            text: body.to_string(),
            server_time: None,
            usage: None,
        }),
        _ => Err(format!("Unsupported response format: {}", response_format).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_transcription_json_text_and_errors() {
        let parsed = parse_transcription(
            "verbose_json",
            r#"{"text":"hi there","inference_time":0.25,"usage":{"type":"duration","seconds":2}}"#,
        )
        .expect("json");
        assert_eq!(parsed.text, "hi there");
        assert_eq!(parsed.server_time, Some(0.25));
        assert_eq!(parsed.usage.expect("usage")["seconds"], 2);
        let legacy = parse_transcription("json", r#"{"transcription":"x","inference_time":-1}"#)
            .expect("legacy field");
        assert_eq!(legacy.text, "x");
        assert_eq!(
            legacy.server_time, None,
            "negative time is not a server time"
        );
        let text = parse_transcription("text", "plain words").expect("text");
        assert_eq!(text.text, "plain words");
        assert!(text.server_time.is_none() && text.usage.is_none());
        assert!(parse_transcription("json", r#"{"other":1}"#).is_err());
        assert!(parse_transcription("json", "not json").is_err());
        assert!(parse_transcription("xml", "x").is_err());
    }

    #[test]
    fn transcription_form_builds_for_every_format() {
        for format in ["verbose_json", "json", "text"] {
            let form = transcription_form(
                "m",
                format,
                "en",
                "a.wav".into(),
                "wav",
                bytes::Bytes::from_static(b"RIFF"),
            )
            .expect("form");
            assert!(!form.boundary().is_empty());
        }
        assert_eq!(mime_for_format("FLAC"), "audio/flac");
        assert_eq!(mime_for_format("unknown"), "audio/mpeg");
    }

    #[test]
    fn loads_audio_samples_and_ground_truth() {
        let dir = tempfile::tempdir().expect("tmp");
        let samples = dir.path().join("audio.jsonl");
        std::fs::write(
            &samples,
            "{\"id\":\"a\",\"path\":\"a.wav\",\"format\":\"wav\",\"duration\":1.5}\n\n{\"id\":\"b\",\"url\":\"http://x/b.mp3\"}\n",
        )
        .expect("write");
        let loaded = load_audio_samples(samples.to_str().unwrap()).expect("samples");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].local_file_path.as_deref(), Some("a.wav"));
        assert_eq!(loaded[0].duration, Some(1.5));
        assert_eq!(loaded[1].format, "unknown");
        assert!(loaded[1].local_file_path.is_none());
        let bad = dir.path().join("bad.jsonl");
        std::fs::write(&bad, "{\"id\":\"c\"}\n").expect("write");
        assert!(load_audio_samples(bad.to_str().unwrap()).is_err());
        let refs = dir.path().join("refs.jsonl");
        std::fs::write(&refs, "{\"id\":\"a\",\"transcript\":\"hello\"}\n").expect("write");
        let truth = load_ground_truth(refs.to_str().unwrap()).expect("refs");
        assert_eq!(truth["a"], "hello");
    }

    #[test]
    fn whisper_normalizer_ignores_case_punctuation_and_fillers() {
        assert_eq!(
            normalize("Hello, [MUSIC] WORLD!", Normalizer::WhisperEnglish),
            "hello world"
        );
        assert_eq!(
            word_error_rate("hello world", "Hello, world.", Normalizer::WhisperEnglish),
            Some(0.0)
        );
    }

    #[test]
    fn english_normalizes_common_contractions_and_numbers() {
        assert_eq!(
            normalize("I can't count one, two.", Normalizer::WhisperEnglish),
            "i can not count 1 2"
        );
    }

    #[test]
    fn wer_and_cer_match_hand_computed_reference_pairs() {
        // (reference, hypothesis, WER, CER) computed by hand on normalized text.
        let table = [
            ("the quick brown fox", "the quick brown fox", 0.0, 0.0),
            // one substitution out of four words; "fox"->"cat" is 3 of 19 chars.
            (
                "the quick brown fox",
                "the quick brown cat",
                0.25,
                3.0 / 19.0,
            ),
            // one deletion out of four words; " fox" is 4 of 19 chars.
            ("the quick brown fox", "the quick brown", 0.25, 4.0 / 19.0),
            // one insertion against three reference words.
            ("the quick fox", "the very quick fox", 1.0 / 3.0, 5.0 / 13.0),
        ];
        for (reference, hypothesis, wer, cer) in table {
            let got_wer = word_error_rate(reference, hypothesis, Normalizer::WhisperEnglish);
            let got_cer = character_error_rate(reference, hypothesis, Normalizer::WhisperEnglish);
            assert!(
                (got_wer.unwrap() - wer).abs() < 1e-9,
                "WER {reference:?} vs {hypothesis:?}: got {got_wer:?}, want {wer}"
            );
            assert!(
                (got_cer.unwrap() - cer).abs() < 1e-9,
                "CER {reference:?} vs {hypothesis:?}: got {got_cer:?}, want {cer}"
            );
        }
    }

    #[test]
    fn normalizer_choice_changes_the_score() {
        let reference = "I can't count one";
        let hypothesis = "i can not count 1";
        assert_eq!(
            word_error_rate(reference, hypothesis, Normalizer::WhisperEnglish),
            Some(0.0)
        );
        // Basic normalization keeps the contraction and the spelled-out
        // numeral: two substitutions and one insertion over four words.
        assert_eq!(
            word_error_rate(reference, hypothesis, Normalizer::WhisperBasic),
            Some(0.75)
        );
        // Raw comparison additionally sees the leading capital.
        assert_eq!(
            word_error_rate(reference, hypothesis, Normalizer::None),
            Some(1.0)
        );
    }

    #[test]
    fn empty_reference_scores_only_when_hypothesis_is_empty() {
        assert_eq!(
            word_error_rate("", "", Normalizer::WhisperEnglish),
            Some(0.0)
        );
        assert_eq!(
            word_error_rate("", "text", Normalizer::WhisperEnglish),
            None
        );
        assert_eq!(
            character_error_rate("", "text", Normalizer::WhisperEnglish),
            None
        );
    }

    #[test]
    fn rtfx_is_audio_over_client_time() {
        assert_eq!(rtfx(10.0, 2.0), Some(5.0));
        assert_eq!(rtfx(10.0, 0.0), None);
    }
}
