// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared endpoint resolution for metrum-ai-bench-cli-llm, metrum-ai-bench-cli-vlm, and metrum-ai-bench-cli-asr.
//! Supports single (--url + --api-key) or multi (--endpoints-file YAML) with weighted round-robin.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type EndpointTuple = (String, String, String);
pub type WeightedEndpoints = Vec<EndpointTuple>;
pub type EndpointWeights = Vec<(String, u32)>;

/// Single endpoint entry from YAML endpoints file.
#[derive(Debug, Clone, Deserialize)]
pub struct EndpointConfig {
    pub url: String,
    pub api_key: String,
    pub name: Option<String>,
    pub weight: Option<u32>,
}

/// Resolved endpoint(s) for the run: either single (url + api_key) or multi (weighted list).
#[derive(Debug, Clone)]
pub enum ResolvedEndpoints {
    Single {
        url: String,
        api_key: String,
        name: String,
    },
    Multi {
        /// (url, api_key, name) repeated by weight for round-robin indexing.
        weighted_list: Vec<(String, String, String)>,
        /// Endpoint names and weights for display (stable order).
        endpoint_names_with_weights: Vec<(String, u32)>,
    },
}

impl ResolvedEndpoints {
    pub fn endpoint_names_for_display(&self) -> Vec<(String, u32)> {
        match self {
            ResolvedEndpoints::Single { name, .. } => vec![(name.clone(), 1)],
            ResolvedEndpoints::Multi {
                endpoint_names_with_weights,
                ..
            } => endpoint_names_with_weights.clone(),
        }
    }

    /// Unique endpoint URLs (for placement warnings).
    pub fn urls(&self) -> Vec<&str> {
        match self {
            ResolvedEndpoints::Single { url, .. } => vec![url.as_str()],
            ResolvedEndpoints::Multi { weighted_list, .. } => {
                let mut out = Vec::new();
                for (url, _, _) in weighted_list {
                    if !out.contains(&url.as_str()) {
                        out.push(url.as_str());
                    }
                }
                out
            }
        }
    }
}

/// Default temporary ejection window after a connect failure (least-inflight).
pub const DEFAULT_CONNECT_EJECT_BACKOFF: Duration = Duration::from_secs(5);

/// Shared endpoint selector. The returned guard decrements the endpoint's
/// in-flight counter on drop, including cancellation and error paths.
#[derive(Debug)]
pub struct EndpointSelector {
    next: AtomicUsize,
    inflight: BTreeMap<String, Arc<AtomicUsize>>,
    /// Temporary ejection deadlines after connect failures (least-inflight).
    ejected_until: BTreeMap<String, Arc<Mutex<Option<Instant>>>>,
    eject_backoff: Duration,
}

#[derive(Debug)]
pub struct EndpointLease {
    counter: Arc<AtomicUsize>,
}

impl Drop for EndpointLease {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

impl EndpointSelector {
    pub fn new(endpoints: &ResolvedEndpoints) -> Self {
        Self::with_eject_backoff(endpoints, DEFAULT_CONNECT_EJECT_BACKOFF)
    }

    pub fn with_eject_backoff(endpoints: &ResolvedEndpoints, eject_backoff: Duration) -> Self {
        let names = endpoints.endpoint_names_for_display();
        let inflight = names
            .iter()
            .map(|(name, _)| (name.clone(), Arc::new(AtomicUsize::new(0))))
            .collect();
        let ejected_until = names
            .into_iter()
            .map(|(name, _)| (name, Arc::new(Mutex::new(None))))
            .collect();
        Self {
            next: AtomicUsize::new(0),
            inflight,
            ejected_until,
            eject_backoff,
        }
    }

    /// Temporarily eject an endpoint after a connect failure so least-inflight
    /// prefers healthy peers for [`DEFAULT_CONNECT_EJECT_BACKOFF`] (or the
    /// configured backoff).
    pub fn note_connect_failure(&self, name: &str) {
        if let Some(slot) = self.ejected_until.get(name) {
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(Instant::now() + self.eject_backoff);
            }
        }
    }

    fn is_ejected(&self, name: &str) -> bool {
        let Some(slot) = self.ejected_until.get(name) else {
            return false;
        };
        let Ok(guard) = slot.lock() else {
            return false;
        };
        match *guard {
            Some(until) => Instant::now() < until,
            None => false,
        }
    }

    pub fn select(
        &self,
        endpoints: &ResolvedEndpoints,
        strategy: crate::args_common::LoadBalancer,
    ) -> ((String, String, String), EndpointLease) {
        let choices: Vec<(String, String, String)> = match endpoints {
            ResolvedEndpoints::Single { url, api_key, name } => {
                vec![(url.clone(), api_key.clone(), name.clone())]
            }
            ResolvedEndpoints::Multi { weighted_list, .. } => weighted_list.clone(),
        };
        let index = match strategy {
            crate::args_common::LoadBalancer::RoundRobin => {
                self.next.fetch_add(1, Ordering::Relaxed) % choices.len()
            }
            crate::args_common::LoadBalancer::LeastInflight => choices
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, _, name))| {
                    let ejected = self.is_ejected(name);
                    let inflight = self
                        .inflight
                        .get(name)
                        .map(|count| count.load(Ordering::Acquire))
                        .unwrap_or(usize::MAX);
                    // Ejected endpoints sort after healthy ones; among equals,
                    // prefer lower in-flight.
                    (u8::from(ejected), inflight)
                })
                .map(|(index, _)| index)
                .unwrap_or(0),
        };
        let choice = choices[index].clone();
        let counter = self
            .inflight
            .get(&choice.2)
            .expect("selector initialized from the same endpoints")
            .clone();
        counter.fetch_add(1, Ordering::AcqRel);
        (choice, EndpointLease { counter })
    }
}

/// Extract hostname from URL for default endpoint name (e.g. https://api.example.com/v1/... -> api.example.com).
pub fn hostname_from_url(url: &str) -> String {
    let after_slash = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    after_slash
        .split('/')
        .next()
        .unwrap_or("default")
        .to_string()
}

/// Load and normalize endpoints from YAML file. Returns weighted list and display list.
pub fn load_endpoints_file(
    path: &str,
) -> Result<(WeightedEndpoints, EndpointWeights), Box<dyn Error + Send + Sync>> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read endpoints file '{}': {}", path, e))?;
    let configs: Vec<EndpointConfig> = serde_yaml::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("Invalid YAML in endpoints file '{}': {}", path, e))?;
    if configs.is_empty() {
        return Err(anyhow::anyhow!(
            "Endpoints file '{}' must contain at least one endpoint",
            path
        )
        .into());
    }
    let mut weighted_list = Vec::new();
    let mut endpoint_names_with_weights = Vec::new();
    for c in configs {
        let url = c.url.trim().to_string();
        let api_key = c.api_key.trim().to_string();
        if url.is_empty() {
            return Err(anyhow::anyhow!("Endpoint has empty url in '{}'", path).into());
        }
        if api_key.is_empty() {
            return Err(anyhow::anyhow!("Endpoint has empty api_key in '{}'", path).into());
        }
        let weight = c.weight.unwrap_or(1).max(1);
        let name = c
            .name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| hostname_from_url(&url));
        for _ in 0..weight {
            weighted_list.push((url.clone(), api_key.clone(), name.clone()));
        }
        endpoint_names_with_weights.push((name, weight));
    }
    Ok((weighted_list, endpoint_names_with_weights))
}

/// Resolve CLI args into ResolvedEndpoints. Errors if neither or both single/multi provided.
pub fn resolve_endpoints(
    url: Option<&str>,
    api_key: Option<&str>,
    endpoints_file: Option<&str>,
) -> Result<ResolvedEndpoints, Box<dyn Error + Send + Sync>> {
    match (url, api_key, endpoints_file) {
        (Some(u), Some(a), None) => Ok(ResolvedEndpoints::Single {
            url: u.to_string(),
            api_key: a.to_string(),
            name: hostname_from_url(u),
        }),
        (None, None, Some(path)) => {
            let (weighted_list, endpoint_names_with_weights) = load_endpoints_file(path)?;
            Ok(ResolvedEndpoints::Multi {
                weighted_list,
                endpoint_names_with_weights,
            })
        }
        (None, None, None) => {
            Err(anyhow::anyhow!("Either provide --url and --api-key, or --endpoints-file").into())
        }
        _ => Err(anyhow::anyhow!(
            "Cannot use --endpoints-file together with --url/--api-key; use one mode only"
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        hostname_from_url, load_endpoints_file, EndpointSelector, ResolvedEndpoints,
        DEFAULT_CONNECT_EJECT_BACKOFF,
    };
    use crate::args_common::LoadBalancer;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn least_inflight_ejects_after_connect_failure() {
        let endpoints = ResolvedEndpoints::Multi {
            weighted_list: vec![
                ("http://a.example/v1".into(), "ka".into(), "alive".into()),
                ("http://b.example/v1".into(), "kb".into(), "dead".into()),
            ],
            endpoint_names_with_weights: vec![("alive".into(), 1), ("dead".into(), 1)],
        };
        let selector = EndpointSelector::with_eject_backoff(&endpoints, Duration::from_secs(60));

        // Pin some in-flight on alive so without ejection, dead would win.
        let (_alive_choice, _alive_lease) =
            selector.select(&endpoints, LoadBalancer::LeastInflight);
        // First pick should be either; force dead to look cheaper by ejecting… wait:
        // After one select, alive has inflight=1. Next least-inflight picks dead.
        let (choice, _lease) = selector.select(&endpoints, LoadBalancer::LeastInflight);
        assert_eq!(choice.2, "dead");

        selector.note_connect_failure("dead");
        // With dead ejected, prefer alive even though it has higher inflight.
        let (choice, _lease) = selector.select(&endpoints, LoadBalancer::LeastInflight);
        assert_eq!(
            choice.2, "alive",
            "connect failure must eject dead from least-inflight"
        );
        let _ = DEFAULT_CONNECT_EJECT_BACKOFF;
    }

    #[test]
    fn test_hostname_from_url() {
        assert_eq!(
            hostname_from_url("https://api.openai.com/v1/chat/completions"),
            "api.openai.com"
        );
        assert_eq!(
            hostname_from_url("http://localhost:8000/v1/chat/completions"),
            "localhost:8000"
        );
        assert_eq!(hostname_from_url("https://host"), "host");
    }

    #[test]
    fn test_load_endpoints_valid_yaml_defaults_name_and_weight() {
        let yaml = r#"
- url: https://api.example.com/v1/chat/completions
  api_key: sk-abc
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();
        let (weighted_list, display) = load_endpoints_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(weighted_list.len(), 1);
        assert_eq!(
            weighted_list[0].0,
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(weighted_list[0].1, "sk-abc");
        assert_eq!(weighted_list[0].2, "api.example.com");
        assert_eq!(display.len(), 1);
        assert_eq!(display[0].0, "api.example.com");
        assert_eq!(display[0].1, 1);
    }

    #[test]
    fn test_load_endpoints_yaml_with_name_and_weight() {
        let yaml = r#"
- url: https://a.example.com/v1
  api_key: key1
  name: prod
  weight: 3
- url: https://b.example.com/v1
  api_key: key2
  weight: 1
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();
        let (weighted_list, display) = load_endpoints_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(weighted_list.len(), 4);
        assert_eq!(weighted_list[0].2, "prod");
        assert_eq!(weighted_list[3].2, "b.example.com");
        assert_eq!(display.len(), 2);
        assert_eq!(display[0], ("prod".to_string(), 3));
        assert_eq!(display[1].0, "b.example.com");
        assert_eq!(display[1].1, 1);
    }

    #[test]
    fn test_load_endpoints_invalid_yaml() {
        let yaml = "not: valid: yaml: [";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();
        let r = load_endpoints_file(f.path().to_str().unwrap());
        assert!(r.is_err());
    }

    #[test]
    fn test_load_endpoints_empty_url_fails() {
        let yaml = r#"
- url: ""
  api_key: sk-x
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();
        let r = load_endpoints_file(f.path().to_str().unwrap());
        assert!(r.is_err());
    }

    #[test]
    fn test_load_endpoints_empty_file_fails() {
        let yaml = "[]";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();
        let r = load_endpoints_file(f.path().to_str().unwrap());
        assert!(r.is_err());
    }
}
