//! Study pipelines: stage configuration, stage keys and provenance records. Specified in
//! `rqm/pipeline/provenance.md`.

mod check;
mod config;
mod external;
mod generate;
mod keys;
mod provenance;
mod qc;
mod rules;
mod summary;

use std::fmt;

pub use check::{
    TEST_KEY_FORMAT, check_key_stale, check_record_clean, check_record_dir, check_record_path,
    check_record_sealed, run_tests, test_files, test_key,
};
pub use config::{
    ExternalConfig, PipelineConfig, StageConfig, StageInput, TemplatePart, config_path,
    load_pipeline, load_variant_lenient, parse_template,
};
pub use external::{
    ExternalError, JOB_DIR, LARGE_INPUTS_BYTES, RECEIPT_FILE, Receipt, SLURM_DEFAULTS_FILE, ingest,
    instructions, job_folder_name, large_inputs_warning, parse_receipt, script_sha256,
    slurm_defaults, stage_external, waiting_jobs, write_job_folder, write_job_folder_with,
};
pub use generate::{generate_rules, snakefile_path, stub, variant_target};
pub use keys::{
    CodeFile, KeyContext, RunPaths, StageKeys, compute_keys, compute_keys_until, items_of,
    output_dir, params_file, render_run, shell_quote, stage_key, stage_key_text,
};
pub use provenance::{
    BegunJob, Cleanliness, ItemSummary, KeyStatus, OutputFile, ProvenanceRecord, RECORD_FILE,
    SealRequest, begin, cleanliness, is_valid_item_name, key_status, platform, seal,
};
pub use qc::{Verdict, VerdictFile, missing_reviews, parse_verdict, qc, qc_status, verdict_path};
pub use rules::pipeline_rules;
pub use summary::{render_summary, stale_entries, summarize, write_summary};

pub(crate) use provenance::ResolvedStage;

// rq-297a1f84
/// Error returned by the pipeline commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    Config {
        path: String,
        line: Option<u32>,
        message: String,
    },
    UnknownVariant(String),
    UnknownStage(String),
    KeyMismatch {
        expected: String,
        actual: String,
    },
    NoOutputs(String),
    BadResult(String),
    ItemsUnsealed(Vec<String>),
    UnconfiguredRoot(String),
    /// Fan-out item names that are duplicated or malformed.
    BadItems(Vec<String>),
    /// A required output has not been sealed under its current key.
    NotSealed(String),
    /// A QC command was refused.
    Qc(String),
    /// A command that needs a human was run without an interactive terminal.
    NotInteractive(String),
    /// A dataset input cannot be used.
    Data(String),
    /// `STUDY.md` is missing or cannot be used.
    Study(String),
    Io(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::Config {
                path,
                line: Some(line),
                message,
            } => write!(f, "{path}:{line}: {message}"),
            PipelineError::Config { path, message, .. } => write!(f, "{path}: {message}"),
            PipelineError::UnknownVariant(v) => write!(f, "unknown variant `{v}`"),
            PipelineError::UnknownStage(s) => write!(f, "unknown stage `{s}`"),
            PipelineError::KeyMismatch { expected, actual } => write!(
                f,
                "the stage key is now {actual}, not {expected}: the configuration or code changed \
                 while the stage ran; run it again"
            ),
            PipelineError::NoOutputs(dir) => {
                write!(f, "the stage produced no output files in {dir}")
            }
            PipelineError::BadResult(m) => write!(f, "bad result file: {m}"),
            PipelineError::ItemsUnsealed(items) => {
                write!(f, "items not sealed yet: {}", items.join(", "))
            }
            PipelineError::UnconfiguredRoot(r) => write!(
                f,
                "root `{r}` is not configured on this machine: set TOPHAT_ROOT_{} or add it to \
                 .tophat/roots.local.yaml",
                r.to_ascii_uppercase()
            ),
            PipelineError::BadItems(items) => write!(
                f,
                "fan-out items with duplicate or invalid names: {}",
                items.join(", ")
            ),
            PipelineError::NotSealed(m) => write!(f, "not sealed: {m}"),
            PipelineError::Qc(m) => write!(f, "{m}"),
            PipelineError::NotInteractive(m) => write!(f, "{m}"),
            PipelineError::Data(m) => write!(f, "{m}"),
            PipelineError::Study(m) => write!(f, "{m}"),
            PipelineError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for PipelineError {}
