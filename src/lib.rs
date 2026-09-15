// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod args_common;
pub mod asr;
pub mod endpoints;
pub mod environment;
pub mod error;
pub mod http_client;
pub mod jsonl;
pub mod load;
pub mod prompt_inputs;
pub mod record;
pub mod runner;
pub mod sse;
pub mod stats;
pub mod strategic;
pub mod summary;
pub mod tokenizer;

pub mod banner {
    pub fn print_banner_metrumbench(version: &str, tool_name: &str) {
        println!(
            r#"
                                                                                                        
        @@@    @@@   @@@@@@@    @@@@@@    @@@@@     @@   @@    @@@    @@@           @@@      @@       
        @@@@  @@@@                @@      @    @@   @@   @@    @@@@  @@@@           @ @@     @@       
        @@ @@@@ @@   @@@@@@@      @@      @@@@@@    @@   @@    @@ @@@@ @@          @@ @@     @@       
        @@  @@  @@                @@      @  @@     @@   @@    @@  @@  @@         @@   @@    @@       
        @@      @@   @@@@@@@      @@      @   @@@     @@@      @@      @@        @@     @@   @@       
                                                                                                        
        AI Performance Testing Tools: {} v{}
        From Metrum AI, Inc. — https://github.com/metrum-ai/bench-cli
        Author: Chetan Gadgil
    "#,
            tool_name, version
        );
    }
}

pub mod timecheck {
    use log::{debug, info, warn};
    use std::net::UdpSocket;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    /// Opt-in SNTP clock check (std-only; no third-party NTP crate).
    /// Returns absolute offset in milliseconds when a server responds.
    /// Logs warnings for large offsets or unreachable servers; never hard-fails.
    pub fn check_ntp_offset() -> Option<i64> {
        let ntp_servers = [
            "pool.ntp.org:123".to_string(),
            "time.google.com:123".to_string(),
            "time.cloudflare.com:123".to_string(),
            "time.apple.com:123".to_string(),
        ];

        let mut all_servers = ntp_servers.to_vec();
        if let Ok(custom_server) = std::env::var("METRUM_NTP_SERVER") {
            debug!("Found custom NTP server in environment: {}", custom_server);
            all_servers.insert(0, custom_server);
        }

        let timeout_secs = std::env::var("METRUM_NTP_TIMEOUT")
            .ok()
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(10);

        let start_time = Instant::now();

        while start_time.elapsed().as_secs() < timeout_secs {
            for server in &all_servers {
                debug!("Attempting SNTP sync check with server: {}", server);
                match query_sntp_offset_ms(server) {
                    Ok(offset_ms) => {
                        if offset_ms > 1000 {
                            warn!(
                                "System clock offset from NTP is {:.3}s (server: {})",
                                offset_ms as f64 / 1000.0,
                                server
                            );
                            println!(
                                "Warning: system clock offset from NTP is {:.3}s (server: {})",
                                offset_ms as f64 / 1000.0,
                                server
                            );
                        } else {
                            info!("NTP clock offset: {}ms (server: {})", offset_ms, server);
                        }
                        return Some(offset_ms);
                    }
                    Err(e) => {
                        warn!("Failed to check NTP time with {}: {}", server, e);
                        continue;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }

        warn!(
            "NTP check could not reach any server within {} seconds",
            timeout_secs
        );
        println!("Warning: NTP check could not determine clock offset (servers unreachable)");
        None
    }

    /// Minimal SNTP client (RFC 5905 client mode). Absolute offset vs local clock.
    fn query_sntp_offset_ms(server: &str) -> Result<i64, String> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;

        // LI=0, VN=4, Mode=3 (client)
        let mut req = [0u8; 48];
        req[0] = 0x23;

        let t1 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        socket.send_to(&req, server).map_err(|e| e.to_string())?;

        let mut resp = [0u8; 48];
        let (n, _) = socket.recv_from(&mut resp).map_err(|e| e.to_string())?;
        if n < 48 {
            return Err(format!("short NTP response ({n} bytes)"));
        }
        let t4 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;

        let mode = resp[0] & 0x07;
        if mode != 4 {
            return Err(format!("unexpected NTP mode {mode}"));
        }

        // Transmit Timestamp (bytes 40..48), NTP epoch -> Unix
        let tx_sec = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]) as i64;
        let tx_frac = u32::from_be_bytes([resp[44], resp[45], resp[46], resp[47]]) as i64;
        let ntp_unix_ms = (tx_sec - 2_208_988_800) * 1000 + (tx_frac * 1000) / 0x1_0000_0000;

        // Approximate offset using mid-point of round trip vs server transmit time.
        let local_mid_ms = ((t1.as_millis() + t4.as_millis()) / 2) as i64;
        Ok((local_mid_ms - ntp_unix_ms).abs())
    }
}

#[cfg(test)]
mod tests {
    use super::unique_id;

    #[test]
    fn unique_id_generation_produces_expected_shape() {
        let id = unique_id::generate_human_readable_unique_id(2);
        let parts: Vec<_> = id.split('-').collect();
        assert!(parts.len() >= 3, "id should be words plus number: {}", id);
        let number = parts.last().expect("number");
        assert!(
            number.parse::<u32>().is_ok(),
            "trailing segment should be numeric: {}",
            id
        );
    }
}

pub mod compile_time_info {
    /// Build identity without embedding wall-clock compile datetime (reproducible).
    pub fn get_compile_info() -> std::collections::HashMap<String, String> {
        let mut info = std::collections::HashMap::new();
        info.insert(
            "rustc_version".to_string(),
            env!("RUSTC_VERSION_STRING").to_string(),
        );
        info
    }
}

pub mod unique_id {
    use rand::prelude::*;
    use uuid::Uuid;

    pub fn generate_uuid() -> String {
        Uuid::new_v4().to_string()
    }

    pub fn generate_human_readable_unique_id(num_words: usize) -> String {
        let words = [
            "alpha",
            "beta",
            "gamma",
            "delta",
            "epsilon",
            "zeta",
            "eta",
            "theta",
            "iota",
            "kappa",
            "lambda",
            "mu",
            "nu",
            "xi",
            "omicron",
            "pi",
            "rho",
            "sigma",
            "tau",
            "upsilon",
            "phi",
            "chi",
            "psi",
            "omega",
            "red",
            "blue",
            "green",
            "yellow",
            "purple",
            "orange",
            "pink",
            "brown",
            "swift",
            "quick",
            "fast",
            "rapid",
            "nimble",
            "agile",
            "fleet",
            "speedy",
            "bright",
            "shiny",
            "radiant",
            "gleaming",
            "brilliant",
            "luminous",
            "glowing",
            "strong",
            "mighty",
            "powerful",
            "robust",
            "sturdy",
            "solid",
            "durable",
            "wise",
            "sage",
            "learned",
            "astute",
            "shrewd",
            "prudent",
            "judicious",
            "brave",
            "bold",
            "daring",
            "valiant",
            "heroic",
            "fearless",
            "courageous",
            "calm",
            "serene",
            "peaceful",
            "tranquil",
            "placid",
            "gentle",
            "quiet",
        ];
        let mut rng = rand::rng();
        let selected: Vec<_> = (0..num_words)
            .map(|_| words.choose(&mut rng).unwrap())
            .collect();
        let number = rng.random_range(1000..10000);
        format!(
            "{}-{}",
            selected
                .into_iter()
                .map(|&s| s.to_string())
                .collect::<Vec<_>>()
                .join("-"),
            number
        )
    }
}
