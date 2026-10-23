// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic fixture generation and offline telemetry-fixture recording.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use flate2::{write::ZlibEncoder, Compression};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const UTTERANCES: &[&str] = &["1089-134686-0030", "2961-961-0016", "6930-81414-0005"];
const TARBALL_URL: &str = "https://www.openslr.org/resources/12/test-clean.tar.gz";
const MD5_URL: &str = "https://www.openslr.org/resources/12/md5sum.txt";

#[derive(Subcommand)]
pub enum FixturesCommand {
    /// Download, verify, and convert the LibriSpeech test-clean ASR fixtures.
    Asr(AsrArgs),
    /// Regenerate the deterministic 512x512 VLM shapes PNG.
    Vlm(VlmArgs),
    /// Re-record the NDJSON/strategic-stdout analysis contract fixtures.
    Analyze(AnalyzeArgs),
}

#[derive(Args)]
pub struct AsrArgs {
    #[arg(long, default_value = "test-data/asr")]
    out: PathBuf,
    #[arg(long, default_value = "")]
    cache: PathBuf,
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(Args)]
pub struct VlmArgs {
    #[arg(long, default_value = "test-data/vlm/shapes-512.png")]
    out: PathBuf,
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(Args)]
pub struct AnalyzeArgs {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, default_value = "test-data/telemetry/analysis")]
    out: PathBuf,
    #[arg(long, default_value = "target/release")]
    bin_dir: PathBuf,
    #[arg(long, default_value_t = 4)]
    capacity: u32,
    #[arg(long, default_value_t = 64)]
    requests_per_stage: u32,
}

fn run_tool(tool: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(tool)
        .args(args)
        .output()
        .with_context(|| format!("run required fixture tool {tool}"))?;
    if !output.status.success() {
        bail!("fixture tool {tool} failed with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn md5(path: &Path) -> Result<String> {
    Ok(
        run_tool("md5sum", &[path.to_str().context("non-UTF8 cache path")?])?
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned(),
    )
}

fn fetch_asr(args: AsrArgs) -> Result<()> {
    let root = args
        .root
        .canonicalize()
        .context("resolve repository root")?;
    let out = if args.out.is_absolute() {
        args.out
    } else {
        root.join(args.out)
    };
    let cache = if args.cache.as_os_str().is_empty() {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join(".cache")
            })
            .join("metrum-ai-bench-cli/librispeech")
    } else if args.cache.is_absolute() {
        args.cache
    } else {
        root.join(args.cache)
    };
    fs::create_dir_all(&cache)?;
    fs::create_dir_all(&out)?;
    for tool in ["curl", "tar", "md5sum", "ffmpeg", "ffprobe"] {
        run_tool(tool, &["--version"]).with_context(|| format!("{tool} is required"))?;
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let md5_text = client.get(MD5_URL).send()?.error_for_status()?.text()?;
    let expected = md5_text
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let checksum = fields.next()?;
            (fields.next()? == "test-clean.tar.gz").then_some(checksum)
        })
        .context("OpenSLR MD5 list does not contain test-clean.tar.gz")?;
    let tarball = cache.join("test-clean.tar.gz");
    if !tarball.is_file() || md5(&tarball)? != expected {
        let status = Command::new("curl")
            .args(["-fL", "--retry", "3", "-C", "-", "-o"])
            .arg(&tarball)
            .arg(TARBALL_URL)
            .status()
            .context("download LibriSpeech archive with curl")?;
        if !status.success() {
            bail!("curl failed downloading LibriSpeech test-clean.tar.gz");
        }
    }
    let actual = md5(&tarball)?;
    if actual != expected {
        bail!("LibriSpeech test-clean.tar.gz MD5 mismatch");
    }
    let work = tempfile::tempdir()?;
    let mut extract = Command::new("tar");
    extract.args([
        "-xzf",
        tarball.to_str().context("non-UTF8 tarball path")?,
        "-C",
        work.path().to_str().context("non-UTF8 temp path")?,
        "--",
    ]);
    let mut members = Vec::new();
    for id in UTTERANCES {
        let mut parts = id.split('-');
        let speaker = parts
            .next()
            .context("invalid static LibriSpeech utterance id")?;
        let chapter = parts
            .next()
            .context("invalid static LibriSpeech utterance id")?;
        let base = format!("LibriSpeech/test-clean/{speaker}/{chapter}");
        members.push(format!("{base}/{id}.flac"));
        members.push(format!("{base}/{speaker}-{chapter}.trans.txt"));
    }
    members.sort();
    members.dedup();
    extract.args(&members);
    let status = extract
        .status()
        .context("extract LibriSpeech fixture members")?;
    if !status.success() {
        bail!("tar failed to extract selected LibriSpeech fixtures");
    }
    let relative_out = out
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let manifest_dir = relative_out.unwrap_or_else(|| out.display().to_string());
    let mut truth = fs::File::create(out.join("truth.jsonl"))?;
    let mut input = fs::File::create(out.join("input.jsonl"))?;
    for id in UTTERANCES {
        let mut parts = id.split('-');
        let speaker = parts
            .next()
            .context("invalid static LibriSpeech utterance id")?;
        let chapter = parts
            .next()
            .context("invalid static LibriSpeech utterance id")?;
        let folder = work
            .path()
            .join(format!("LibriSpeech/test-clean/{speaker}/{chapter}"));
        let source = folder.join(format!("{id}.flac"));
        let destination = out.join(format!("{id}.wav"));
        let status = Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
            .arg(&source)
            .args([
                "-map_metadata",
                "-1",
                "-fflags",
                "+bitexact",
                "-flags:a",
                "+bitexact",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-sample_fmt",
                "s16",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&destination)
            .status()
            .context("convert LibriSpeech FLAC to WAV")?;
        if !status.success() {
            bail!("ffmpeg failed converting {id}");
        }
        let transcript_path = folder.join(format!("{speaker}-{chapter}.trans.txt"));
        let transcript = fs::read_to_string(transcript_path)?
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{id} ")))
            .context("LibriSpeech transcript is missing an utterance")?
            .to_owned();
        if transcript.trim().is_empty() {
            bail!("LibriSpeech transcript is empty for {id}");
        }
        let duration = run_tool(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
                destination.to_str().context("non-UTF8 output path")?,
            ],
        )?
        .parse::<f64>()
        .context("parse ffprobe duration")?;
        writeln!(
            truth,
            "{{\"id\":{},\"transcript\":{}}}",
            serde_json::to_string(id)?,
            serde_json::to_string(&transcript)?
        )?;
        writeln!(
            input,
            "{{\"id\":{},\"path\":{},\"format\":\"wav\",\"duration\":{duration:.3}}}",
            serde_json::to_string(id)?,
            serde_json::to_string(&format!("{manifest_dir}/{id}.wav"))?
        )?;
        println!("wrote {} ({duration:.3}s)", destination.display());
    }
    Ok(())
}

const SIZE: usize = 512;
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [20, 20, 20];
const RED: [u8; 3] = [220, 40, 40];
const BLUE: [u8; 3] = [40, 80, 220];
const GREEN: [u8; 3] = [30, 160, 70];
const GLYPHS: [(&str, [&str; 7]); 5] = [
    (
        "B",
        [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
    ),
    (
        "E",
        [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
    ),
    (
        "N",
        [
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
    ),
    (
        "C",
        [
            ".####", "#....", "#....", "#....", "#....", "#....", ".####",
        ],
    ),
    (
        "H",
        [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
    ),
];

fn png_chunk(kind: &[u8; 4], data: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_data = Vec::with_capacity(4 + data.len());
    crc_data.extend_from_slice(kind);
    crc_data.extend_from_slice(data);
    output.extend_from_slice(&crc32fast::hash(&crc_data).to_be_bytes());
}

fn vlm_png() -> Result<Vec<u8>> {
    let mut pixels = vec![WHITE; SIZE * SIZE];
    let (cx, cy, radius) = (128_i32, 140_i32, 80_i32);
    for y in cy - radius..=cy + radius {
        for x in cx - radius..=cx + radius {
            if (x - cx).pow(2) + (y - cy).pow(2) <= radius.pow(2) {
                pixels[y as usize * SIZE + x as usize] = RED;
            }
        }
    }
    for y in 64..224 {
        for x in 304..464 {
            pixels[y * SIZE + x] = BLUE;
        }
    }
    for y in 250..=400 {
        let half = (y - 250) * 110 / 150;
        for x in 256 - half..=256 + half {
            pixels[y * SIZE + x] = GREEN;
        }
    }
    let scale = 8;
    let word = "BENCH";
    let width = word.len() * 6 * scale - scale;
    let x0 = (SIZE - width) / 2;
    for (index, ch) in word.chars().enumerate() {
        let rows = &GLYPHS
            .iter()
            .find(|(letter, _)| letter.starts_with(ch))
            .context("missing VLM fixture glyph")?
            .1;
        for (gy, row) in rows.iter().enumerate() {
            for (gx, byte) in row.bytes().enumerate() {
                if byte != b'#' {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = x0 + (index * 6 + gx) * scale + dx;
                        let y = 424 + gy * scale + dy;
                        pixels[y * SIZE + x] = BLACK;
                    }
                }
            }
        }
    }
    let mut raw = Vec::with_capacity(SIZE * (1 + 3 * SIZE));
    for row in pixels.chunks_exact(SIZE) {
        raw.push(0);
        for pixel in row {
            raw.extend_from_slice(pixel);
        }
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&raw)?;
    let compressed = encoder.finish()?;
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(SIZE as u32).to_be_bytes());
    ihdr.extend_from_slice(&(SIZE as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(b"IHDR", &ihdr, &mut png);
    png_chunk(b"IDAT", &compressed, &mut png);
    png_chunk(b"IEND", &[], &mut png);
    Ok(png)
}

fn generate_vlm(args: VlmArgs) -> Result<()> {
    let root = args.root.canonicalize()?;
    let out = if args.out.is_absolute() {
        args.out
    } else {
        root.join(args.out)
    };
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out, vlm_png()?)?;
    println!("wrote {}", out.display());
    Ok(())
}

struct MockProcess(Child);

impl Drop for MockProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn start_mock(binary: &Path, port: u16, capacity: Option<u32>) -> Result<MockProcess> {
    let mut command = Command::new(binary);
    command.args([
        "--listen",
        &format!("127.0.0.1:{port}"),
        "--telemetry-fixture",
        "--latency-ms",
        "200",
        "--fail-every",
        "7",
    ]);
    if let Some(capacity) = capacity {
        command.args(["--capacity", &capacity.to_string()]);
    }
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start mock fixture server")?;
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if client
            .get(&url)
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(MockProcess(child));
        }
        thread::sleep(Duration::from_millis(50));
    }
    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();
    bail!("mock fixture server failed to become ready")
}

fn normalize_value(value: &mut Value, run_id: &str, mock_url: &str, gate_url: Option<&str>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if key == "run_id" && child.as_str() == Some(run_id) {
                    *child = Value::String("RUN_ID".into());
                } else if key == "t0_wall" {
                    *child = Value::String("T0_WALL".into());
                } else {
                    normalize_value(child, run_id, mock_url, gate_url);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| normalize_value(item, run_id, mock_url, gate_url)),
        Value::String(text) => {
            *text = text.replace(mock_url, "http://127.0.0.1:MOCK_PORT");
            if let Some(gate) = gate_url {
                *text = text.replace(gate, "http://127.0.0.1:GATE_PORT");
            }
        }
        _ => {}
    }
}

struct RecordConfig<'a> {
    binary: &'a Path,
    telemetry: &'a Path,
    requests: u32,
    work: &'a Path,
    output_dir: &'a Path,
}

fn record_one(
    name: &str,
    sweep: &str,
    target_url: &str,
    mock_url: &str,
    gate_url: Option<&str>,
    config: &RecordConfig<'_>,
) -> Result<()> {
    let binary = config.binary;
    let telemetry = config.telemetry;
    let requests = config.requests;
    let work = config.work;
    let output_dir = config.output_dir;
    let ndjson = work.join(format!("{name}.ndjson"));
    let stdout = work.join(format!("{name}.stdout.json"));
    let output = Command::new(binary)
        .args([
            "--url",
            &format!("{target_url}/v1/chat/completions"),
            "--model",
            "mock",
            "--api-key",
            "dummy",
            "--sweep",
            sweep,
            "--sweep-by",
            "concurrency",
            "--requests-per-stage",
            &requests.to_string(),
            "--max-tokens",
            "16",
            "--warmup-requests",
            "0",
            "--ndjson",
            ndjson.to_str().context("non-UTF8 NDJSON path")?,
            "--telemetry",
            telemetry.to_str().context("non-UTF8 telemetry path")?,
            "--metrics-url",
            &format!("{mock_url}/metrics"),
        ])
        .current_dir(work)
        .output()
        .context("run strategic fixture sweep")?;
    if !output.status.success() {
        bail!(
            "strategic fixture sweep {name} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(&stdout, &output.stdout)?;
    let lines = fs::read_to_string(&ndjson)?;
    let first: Value =
        serde_json::from_str(lines.lines().next().context("NDJSON output is empty")?)?;
    let run_id = first["run_id"]
        .as_str()
        .context("first NDJSON row has no run_id")?
        .to_owned();
    let mut normalized_rows = String::new();
    let mut request_rows = 0;
    let mut stage_rows = 0;
    for line in lines.lines().filter(|line| !line.trim().is_empty()) {
        let mut row: Value = serde_json::from_str(line)?;
        request_rows += usize::from(row["kind"] == "request");
        stage_rows += usize::from(row["kind"] == "stage");
        normalize_value(&mut row, &run_id, mock_url, gate_url);
        normalized_rows.push_str(&serde_json::to_string(&row)?);
        normalized_rows.push('\n');
    }
    fs::write(output_dir.join(format!("{name}.ndjson")), &normalized_rows)?;
    let mut summary: Value =
        serde_json::from_slice(&output.stdout).context("parse strategic summary JSON")?;
    let mut normalized_summary = serde_json::Map::new();
    let source = summary
        .as_object_mut()
        .context("strategic summary is not an object")?;
    for key in [
        "schema_version",
        "tool_version",
        "partial",
        "points",
        "knee",
        "knee_detection",
    ] {
        if let Some(value) = source.remove(key) {
            normalized_summary.insert(key.to_owned(), value);
        }
    }
    let mut normalized_summary = Value::Object(normalized_summary);
    normalize_value(&mut normalized_summary, &run_id, mock_url, gate_url);
    if let Some(points) = normalized_summary["points"].as_array_mut() {
        for point in points {
            if let Some(object) = point.as_object_mut() {
                object.retain(|key, _| {
                    ["load", "p95_s", "throughput", "error_rate"].contains(&key.as_str())
                });
            }
        }
    }
    if !normalized_summary["knee"].is_null() {
        let load = normalized_summary["knee"]["load"].clone();
        normalized_summary["knee"] = json!({"load":load});
    }
    fs::write(
        output_dir.join(format!("{name}.stdout.json")),
        format!("{}\n", serde_json::to_string_pretty(&normalized_summary)?),
    )?;
    let points = normalized_summary["points"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    if points == 0 || request_rows == 0 || stage_rows == 0 || normalized_rows.is_empty() {
        bail!("fixture sweep {name} generated no points, request rows, stages, or NDJSON");
    }
    println!("recorded {name}: {points} points, {stage_rows} stages, {request_rows} requests");
    Ok(())
}

fn validate_analysis_recording(output_dir: &Path) -> Result<()> {
    let summary = |name: &str| -> Result<Value> {
        Ok(serde_json::from_slice(&fs::read(
            output_dir.join(format!("{name}.stdout.json")),
        )?)?)
    };
    let bent = summary("sweep5")?;
    let points = bent["points"].as_array().context("sweep5 has no points")?;
    let first = points.first().context("sweep5 has no first point")?["p95_s"]
        .as_f64()
        .context("sweep5 first p95 is missing")?;
    let last = points.last().context("sweep5 has no last point")?["p95_s"]
        .as_f64()
        .context("sweep5 last p95 is missing")?;
    if points.len() < 5 || last / first < 1.2 {
        bail!("recorded sweep5 does not have a real 20% p95 bend");
    }
    let flat = summary("sweep5_no_bend")?;
    let points = flat["points"]
        .as_array()
        .context("sweep5_no_bend has no points")?;
    let first = points
        .first()
        .context("sweep5_no_bend has no first point")?["p95_s"]
        .as_f64()
        .context("sweep5_no_bend first p95 is missing")?;
    let last = points.last().context("sweep5_no_bend has no last point")?["p95_s"]
        .as_f64()
        .context("sweep5_no_bend last p95 is missing")?;
    if points.len() < 5 || last / first >= 1.2 {
        bail!("recorded sweep5_no_bend is not flat within 20% p95");
    }
    let short = summary("sweep3")?;
    if short["points"].as_array().map(Vec::len) != Some(3) {
        bail!("recorded sweep3 does not contain exactly three points");
    }
    println!("analysis fixtures validated: sweep5 bends, sweep5_no_bend stays flat, sweep3 has three points");
    Ok(())
}

fn record_analysis(args: AnalyzeArgs) -> Result<()> {
    if args.capacity == 0 || args.requests_per_stage == 0 {
        bail!("capacity and requests-per-stage must both be greater than zero");
    }
    let root = args.root.canonicalize()?;
    let bin_dir = if args.bin_dir.is_absolute() {
        args.bin_dir
    } else {
        root.join(args.bin_dir)
    };
    let mock_bin = bin_dir.join("metrum-ai-bench-cli-mock-server");
    let strategic = bin_dir.join("metrum-ai-bench-cli-strategic");
    for binary in [&mock_bin, &strategic] {
        if !binary.is_file() {
            bail!(
                "fixture recording requires prebuilt binary {}",
                binary.display()
            );
        }
    }
    let mock_port = free_port()?;
    let _mock = start_mock(&mock_bin, mock_port, None)?;
    let gate_port = free_port()?;
    let _gate = start_mock(&mock_bin, gate_port, Some(args.capacity))?;
    let mock_url = format!("http://127.0.0.1:{mock_port}");
    let gate_url = format!("http://127.0.0.1:{gate_port}");
    let output_dir = if args.out.is_absolute() {
        args.out
    } else {
        root.join(args.out)
    };
    fs::create_dir_all(&output_dir)?;
    let work = tempfile::tempdir()?;
    let template =
        fs::read_to_string(root.join("test-data/telemetry/analysis/mock-telemetry.yaml"))?;
    let telemetry = work.path().join("telemetry.yaml");
    fs::write(&telemetry, template.replace("MOCK_URL", &mock_url))?;
    let config = RecordConfig {
        binary: &strategic,
        telemetry: &telemetry,
        requests: args.requests_per_stage,
        work: work.path(),
        output_dir: &output_dir,
    };
    record_one(
        "sweep5",
        "1,2,4,8,16",
        &gate_url,
        &mock_url,
        Some(&gate_url),
        &config,
    )?;
    record_one(
        "sweep5_no_bend",
        "1,2,4,8,16",
        &mock_url,
        &mock_url,
        None,
        &config,
    )?;
    record_one("sweep3", "1,4,16", &mock_url, &mock_url, None, &config)?;
    validate_analysis_recording(&output_dir)?;
    Ok(())
}

pub fn run(command: FixturesCommand) -> Result<()> {
    match command {
        FixturesCommand::Asr(args) => fetch_asr(args),
        FixturesCommand::Vlm(args) => generate_vlm(args),
        FixturesCommand::Analyze(args) => record_analysis(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn decoded_png(data: &[u8]) -> Vec<u8> {
        let mut decoder = png::Decoder::new(Cursor::new(data));
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut output).unwrap();
        output.truncate(frame.buffer_size());
        output
    }

    #[test]
    fn vlm_fixture_is_deterministic_and_pixel_identical_to_checked_in_png() {
        let first = vlm_png().unwrap();
        let second = vlm_png().unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(u32::from_be_bytes(first[16..20].try_into().unwrap()), 512);
        assert_eq!(u32::from_be_bytes(first[20..24].try_into().unwrap()), 512);
        assert_eq!(
            decoded_png(&first),
            decoded_png(include_bytes!("../../test-data/vlm/shapes-512.png"))
        );
    }

    #[test]
    fn selected_librispeech_utterances_are_nonempty() {
        assert_eq!(UTTERANCES.len(), 3);
        assert!(UTTERANCES.iter().all(|id| id.split('-').count() == 3));
    }
}
