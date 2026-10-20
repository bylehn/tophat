//! Study pipelines: stage configuration, stage keys and provenance records. Specified in
//! `rqm/pipeline/provenance.md`.

mod config;
mod keys;
mod provenance;
mod rules;

use std::fmt;

pub use config::{
    config_path, load_pipeline, parse_template, PipelineConfig, StageConfig, StageInput,
    TemplatePart,
};
pub use keys::{
    compute_keys, items_of, output_dir, params_file, render_run, shell_quote, stage_key,
    stage_key_text, CodeFile, KeyContext, RunPaths, StageKeys,
};
pub use provenance::{
    cleanliness, is_valid_item_name, key_status, platform, seal, Cleanliness, ItemSummary,
    KeyStatus, OutputFile, ProvenanceRecord, SealRequest, RECORD_FILE,
};
pub use rules::pipeline_rules;

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
            PipelineError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for PipelineError {}
