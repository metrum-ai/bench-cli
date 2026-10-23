// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Resolve prebuilt benchmark executables without compiling them.

use anyhow::{bail, Result};
use std::{
    env,
    path::{Path, PathBuf},
};

pub fn resolve(root: &Path, name: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = env::var_os("BENCH_BIN_DIR") {
        candidates.push(PathBuf::from(dir).join(name));
    }
    candidates.extend([
        root.join("bin").join(name),
        root.join("target/release").join(name),
        root.join("target/rel-user/release").join(name),
    ]);
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|dir| dir.join(name)));
    }
    for candidate in candidates {
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    bail!("{name} not found (checked BENCH_BIN_DIR, {}, target/release, target/rel-user/release, PATH); unpack a prebuilt release or build once in the primary checkout", root.join("bin").display())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn search_order_prefers_release_locations_and_never_debug() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("target/release/bench");
        let rel_user = root.path().join("target/rel-user/release/bench");
        let debug = root.path().join("target/debug/bench");
        executable(&release);
        executable(&rel_user);
        executable(&debug);
        assert_eq!(resolve(root.path(), "bench").unwrap(), release);
    }

    #[test]
    fn explicit_directory_has_highest_priority() {
        let root = tempfile::tempdir().unwrap();
        let explicit = tempfile::tempdir().unwrap();
        let override_bin = explicit.path().join("bench");
        let bundled = root.path().join("bin/bench");
        executable(&override_bin);
        executable(&bundled);
        let old = env::var_os("BENCH_BIN_DIR");
        env::set_var("BENCH_BIN_DIR", explicit.path());
        let result = resolve(root.path(), "bench").unwrap();
        if let Some(old) = old {
            env::set_var("BENCH_BIN_DIR", old);
        } else {
            env::remove_var("BENCH_BIN_DIR");
        }
        assert_eq!(result, override_bin);
    }

    #[test]
    fn missing_binary_fails_instead_of_building() {
        let root = tempfile::tempdir().unwrap();
        let error = resolve(root.path(), "missing-bench").unwrap_err();
        assert!(error.to_string().contains("prebuilt release"));
    }
}
