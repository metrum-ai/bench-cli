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
    use ntp::request;

    /// Opt-in NTP clock check. Returns absolute offset in milliseconds when a
    /// server responds. Logs warnings for large offsets or unreachable servers;
    /// never hard-fails.
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

        let start_time = std::time::Instant::now();

        while start_time.elapsed().as_secs() < timeout_secs {
            for server in &all_servers {
                debug!("Attempting NTP sync check with server: {}", server);
                match request(server) {
                    Ok(packet) => {
                        let ntp_time = ((packet.transmit_time.sec as i64 - 2208988800) * 1000)
                            + (packet.transmit_time.frac as i64 * 1000 / 0x100000000);
                        let system_time = chrono::Utc::now().timestamp_millis();
                        let offset_ms = (system_time - ntp_time).abs();

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
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        warn!(
            "NTP check could not reach any server within {} seconds",
            timeout_secs
        );
        println!("Warning: NTP check could not determine clock offset (servers unreachable)");
        None
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
    pub fn get_compile_info() -> std::collections::HashMap<String, String> {
        let mut info = std::collections::HashMap::new();
        info.insert(
            "compile_datetime".to_string(),
            compile_time::datetime_str!().to_string(),
        );
        info.insert(
            "rustc_version".to_string(),
            compile_time::rustc_version_str!().to_string(),
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
