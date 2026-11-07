// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//! Explicit seed/ordered-stage mapping; reserve new outputs before traffic.
use crate::{
    kv_study_export::{PhaseRecords, StudyExport, StudySeed},
    record::RequestRecord,
    strategic::BenchRecord,
};
use anyhow::{ensure, Context, Result};
use clap::Args;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Args, Debug, Clone, Default)]
pub struct StudyExportArgs {
    #[arg(
        long,
        requires = "kv_study_prefix",
        help = "Opt-in unverified study declaration; measured phases map to planned stages in array order"
    )]
    pub kv_study_manifest: Option<PathBuf>,
    #[arg(
        long,
        requires = "kv_study_manifest",
        help = "New artifact prefix for .study.json/.study.csv/.study.html; never overwrite existing files"
    )]
    pub kv_study_prefix: Option<PathBuf>,
}
struct Output {
    file: File,
    path: PathBuf,
}
pub struct StudyExportPlan {
    seed: Option<StudySeed>,
    ids: Vec<String>,
    outputs: Vec<Output>,
    committed: bool,
}
fn identity(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok(parent
        .canonicalize()?
        .join(path.file_name().context("study_output_filename_required")?))
}
impl StudyExportArgs {
    pub fn prepare(
        &self,
        expected: usize,
        protected: &[PathBuf],
        policy: crate::cache_workload::CachePolicy,
        order: crate::cache_workload::OrderPolicy,
    ) -> Result<Option<StudyExportPlan>> {
        let Some(manifest) = &self.kv_study_manifest else {
            ensure!(self.kv_study_prefix.is_none(), "study_manifest_required");
            return Ok(None);
        };
        let prefix = self
            .kv_study_prefix
            .as_ref()
            .context("study_prefix_required")?;
        let mut bytes = Vec::new();
        File::open(manifest)
            .context("study_manifest_open_failed")?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        let seed = StudySeed::parse(&bytes)?;
        seed.check_controls(policy, order)?;
        let ids = seed.measured_phase_ids()?;
        ensure!(
            ids.len() == expected,
            "study_planned_stage_binding_mismatch"
        );
        let mut plan = StudyExportPlan {
            seed: Some(seed),
            ids,
            outputs: Vec::new(),
            committed: false,
        };
        for extension in [".study.json", ".study.csv", ".study.html"] {
            let mut name = prefix.as_os_str().to_os_string();
            name.push(extension);
            let path = PathBuf::from(name);
            let target = identity(&path)?;
            ensure!(
                protected
                    .iter()
                    .chain(std::iter::once(manifest))
                    .all(|p| identity(p).map(|p| p != target).unwrap_or(true)),
                "study_output_alias"
            );
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .context("study_output_reservation_failed")?;
            plan.outputs.push(Output { file, path });
        }
        Ok(Some(plan))
    }
}
impl Drop for StudyExportPlan {
    fn drop(&mut self) {
        if !self.committed {
            for output in self.outputs.drain(..) {
                drop(output.file);
                let _ = std::fs::remove_file(&output.path);
            }
        }
    }
}
impl StudyExportPlan {
    fn write(&mut self, export: StudyExport) -> Result<()> {
        // Materialize every view before writing any destination. Partial failed
        // writes are removed by Drop, never left as apparently complete exports.
        let views = [export.json()?, export.csv()?, export.html()?.into_bytes()];
        for (output, bytes) in self.outputs.iter_mut().zip(views) {
            output.file.write_all(&bytes)?;
            output.file.sync_all()?;
        }
        self.committed = true;
        Ok(())
    }
    pub fn requests(
        mut self,
        rows: &[RequestRecord],
        native: &[serde_json::Value],
        runtime: &serde_json::Value,
    ) -> Result<()> {
        let bindings = BTreeMap::from([(self.ids[0].clone(), PhaseRecords::Requests(rows))]);
        let mut export = StudyExport::bind(self.seed.take().expect("one use seed"), &bindings)?;
        export.attach_native(native)?;
        export.attach_runtime(runtime)?;
        self.write(export)
    }
    pub fn strategic(
        mut self,
        rows: &[BenchRecord],
        stages: &[f64],
        native: &[serde_json::Value],
        runtime: &serde_json::Value,
    ) -> Result<()> {
        ensure!(
            stages.len() == self.ids.len(),
            "study_stage_mapping_changed"
        );
        ensure!(
            rows.iter().all(|r| stages.contains(&r.stage)),
            "study_unmapped_record_stage"
        );
        let groups: Vec<Vec<_>> = stages
            .iter()
            .map(|load| rows.iter().filter(|r| r.stage == *load).collect())
            .collect();
        let bindings = BTreeMap::from_iter(
            self.ids
                .iter()
                .zip(&groups)
                .map(|(id, rows)| (id.clone(), PhaseRecords::StrategicRefs(rows))),
        );
        let mut export = StudyExport::bind(self.seed.take().expect("one use seed"), &bindings)?;
        export.attach_native(native)?;
        export.attach_runtime(runtime)?;
        self.write(export)
    }
}
