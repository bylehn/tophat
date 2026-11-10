//! Studies: the `STUDY.md` document, its lifecycle, the generated `SUMMARY.md`, the `study` check
//! rule family, and the `tophat study` commands. Specified in `rqm/study/format.md`.

mod commands;
mod document;
mod edit;
mod exploration;
mod fingerprint;
mod rules;
mod summary;

use std::fmt;

use crate::check::Diagnostic;

pub use crate::markdown::Section;
pub use commands::{
    Console, link, migrate, new_study, preregister, record_exploration, replicate, report,
    supersede,
};
pub use document::{
    Amendment, Field, FrontMatter, Preregistration, Scenario, Study, TagIssue, parse_study,
    parse_study_text,
};
pub use exploration::{
    EXPLORATION_FILE, Exploration, TrialResult, exploration_hash, explore, parse_exploration,
    write_exploration,
};
pub use fingerprint::frozen_fingerprint;
pub use rules::{study_rules, study_rules_at};
pub use summary::{
    Epistemic, ResultLine, ResultTag, Summary, TagProblem, classify_tag, parse_summary,
};

/// Directory, relative to the project root, that holds one subdirectory per study.
pub const STUDIES_DIR: &str = "studies";
/// File name of the study document.
pub const STUDY_FILE: &str = "STUDY.md";
/// File name of the generated summary.
pub const SUMMARY_FILE: &str = "SUMMARY.md";

/// The governed `##` sections of `STUDY.md` for a mode, in template order.
pub fn governed_sections(mode: Mode) -> Vec<&'static str> {
    let mut sections = vec!["Question"];
    sections.extend(mode.mode_sections());
    sections.extend(["Variants", "Stages"]);
    if mode.preregisters() {
        sections.push("Amendments");
    }
    sections.extend(["Result", "Interpretation"]);
    sections
}

// rq-08c7718c rq-597a5aff
/// The sections frozen by pre-registration, in fingerprint order.
pub fn frozen_sections(mode: Mode) -> &'static [&'static str] {
    match mode {
        Mode::Confirmatory => &["Question", "Primary endpoint", "Decision rule", "Scenarios"],
        Mode::Descriptive => &["Question", "Deliverables", "Scenarios"],
        Mode::Methods => &["Question", "Success criteria", "Scenarios"],
        Mode::Replication => &["Question", "Match criterion", "Scenarios"],
        Mode::Exploratory | Mode::Retrospective => &[],
    }
}

// rq-597a5aff
/// How a study produces knowledge. Orthogonal to its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Confirmatory,
    Exploratory,
    Descriptive,
    Methods,
    Replication,
    Retrospective,
}

impl Mode {
    /// Parses the front-matter spelling of a mode.
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "confirmatory" => Mode::Confirmatory,
            "exploratory" => Mode::Exploratory,
            "descriptive" => Mode::Descriptive,
            "methods" => Mode::Methods,
            "replication" => Mode::Replication,
            "retrospective" => Mode::Retrospective,
            _ => return None,
        })
    }

    /// The front-matter spelling of the mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Confirmatory => "confirmatory",
            Mode::Exploratory => "exploratory",
            Mode::Descriptive => "descriptive",
            Mode::Methods => "methods",
            Mode::Replication => "replication",
            Mode::Retrospective => "retrospective",
        }
    }

    /// Whether the mode has a `preregistered` state and freezes a subset of the plan.
    pub fn preregisters(self) -> bool {
        !matches!(self, Mode::Exploratory | Mode::Retrospective)
    }

    /// The mode-specific sections (besides `Question` and `Result`).
    pub fn mode_sections(self) -> &'static [&'static str] {
        match self {
            Mode::Confirmatory => &["Primary endpoint", "Decision rule", "Scenarios"],
            Mode::Descriptive => &["Deliverables", "Scenarios"],
            Mode::Methods => &["Success criteria", "Scenarios"],
            Mode::Replication => &["Match criterion", "Scenarios"],
            Mode::Exploratory | Mode::Retrospective => &[],
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// rq-5d9d0d62
/// Lifecycle status of a study.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Draft,
    Preregistered,
    Reported,
    Superseded,
}

impl Status {
    /// Parses the front-matter spelling of a status. `retrospective` is the pre-mode form and is
    /// reported separately by the rules.
    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "draft" => Status::Draft,
            "preregistered" => Status::Preregistered,
            "reported" => Status::Reported,
            "superseded" => Status::Superseded,
            _ => return None,
        })
    }

    /// The front-matter spelling of the status.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Preregistered => "preregistered",
            Status::Reported => "reported",
            Status::Superseded => "superseded",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// rq-7d5a35b9
/// A study-to-study relation recorded on the relating study.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relation {
    FollowsUp,
    Reanalyses,
    Refutes,
    Extends,
    DerivesFrom,
}

impl Relation {
    /// The front-matter field holding the relation.
    pub fn field(self) -> &'static str {
        match self {
            Relation::FollowsUp => "follows_up",
            Relation::Reanalyses => "reanalyses",
            Relation::Refutes => "refutes",
            Relation::Extends => "extends",
            Relation::DerivesFrom => "derived_from",
        }
    }
}

// rq-dba2a6b7
/// Error returned by study parsing and the `tophat study` commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudyError {
    InvalidStudyId(String),
    InvalidQuestionId(String),
    StudyExists(String),
    StudyNotFound(String),
    /// The front matter is missing, not valid YAML, or a field is missing or mistyped.
    FrontMatter {
        path: String,
        line: Option<u32>,
        message: String,
    },
    /// A section a command needs is missing or duplicated.
    Structure {
        path: String,
        message: String,
    },
    WrongStatus {
        study: String,
        status: Option<Status>,
        required: Vec<Status>,
    },
    /// A command that does not apply to the study's mode.
    WrongMode {
        study: String,
        mode: Mode,
    },
    /// The `mode` value is not one of the allowed modes.
    ModeInvalid(String),
    /// A malformed or unusable `reproduces` field.
    ReproducesInvalid(String),
    /// A malformed, missing or self-referential relation.
    LinkInvalid {
        study: String,
        relation: Relation,
        other: String,
    },
    /// `migrate` on a study it cannot convert.
    Migration(String),
    /// A study cannot supersede itself.
    SameStudy(String),
    NotInteractive,
    ConfirmationMismatch,
    NotReady(Vec<Diagnostic>),
    /// The study is past its draft, so its exploration record is locked.
    ExplorationLocked(String),
    Summary(String),
    Io(String),
}

impl fmt::Display for StudyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StudyError::InvalidStudyId(id) => write!(
                f,
                "invalid study ID `{id}`: use 3 to 64 lowercase letters, digits and single hyphens"
            ),
            StudyError::InvalidQuestionId(id) => write!(
                f,
                "invalid question ID `{id}`: expected `q-` followed by 8 lowercase hex characters"
            ),
            StudyError::StudyExists(id) => write!(f, "study `{id}` already exists"),
            StudyError::StudyNotFound(id) => write!(f, "study `{id}` does not exist"),
            StudyError::FrontMatter {
                path,
                line,
                message,
            } => match line {
                Some(line) => write!(f, "{path}:{line}: {message}"),
                None => write!(f, "{path}: {message}"),
            },
            StudyError::Structure { path, message } => write!(f, "{path}: {message}"),
            StudyError::WrongStatus {
                study,
                status,
                required,
            } => {
                let required: Vec<_> = required.iter().map(|s| s.as_str()).collect();
                let status = status.map_or("missing or invalid", |s| s.as_str());
                write!(
                    f,
                    "study `{study}` has status `{status}`; this requires status {}",
                    required.join(" or ")
                )
            }
            StudyError::WrongMode { study, mode } => write!(
                f,
                "study `{study}` is in `{mode}` mode, which has no plan to lock; this command does \
                 not apply to it"
            ),
            StudyError::ModeInvalid(mode) => write!(
                f,
                "`{mode}` is not a valid study mode; use confirmatory, exploratory, descriptive, \
                 methods, replication or retrospective"
            ),
            StudyError::ReproducesInvalid(message) => write!(f, "reproduces: {message}"),
            StudyError::LinkInvalid {
                study,
                relation,
                other,
            } => write!(
                f,
                "study `{study}` cannot record `{}` of `{other}`: the target must be an existing \
                 study other than `{study}`",
                relation.field()
            ),
            StudyError::Migration(message) => write!(f, "cannot migrate: {message}"),
            StudyError::SameStudy(id) => write!(f, "study `{id}` cannot supersede itself"),
            StudyError::NotInteractive => write!(
                f,
                "pre-registration must be done by a human in an interactive terminal; \
                 run this command yourself"
            ),
            StudyError::ConfirmationMismatch => {
                write!(
                    f,
                    "confirmation did not match the study ID; nothing was changed"
                )
            }
            StudyError::NotReady(diagnostics) => {
                writeln!(f, "the study is not ready for pre-registration:")?;
                for d in diagnostics {
                    let line = d.line.map(|l| format!(":{l}")).unwrap_or_default();
                    writeln!(f, "  {}{line}: [{}] {}", d.path, d.rule, d.message)?;
                }
                Ok(())
            }
            StudyError::ExplorationLocked(id) => write!(
                f,
                "study `{id}` is no longer a draft: its exploration was recorded in \
                 EXPLORATION.md when the plan was locked"
            ),
            StudyError::Summary(message) => write!(f, "SUMMARY.md: {message}"),
            StudyError::Io(message) => write!(f, "I/O error: {message}"),
        }
    }
}

impl std::error::Error for StudyError {}

impl From<std::io::Error> for StudyError {
    fn from(e: std::io::Error) -> Self {
        StudyError::Io(e.to_string())
    }
}

/// Whether `s` matches `^[a-z0-9]+(-[a-z0-9]+)*$` (the pattern for study IDs, variants and stages).
pub fn matches_id_pattern(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

// rq-76ac5b2e
/// Whether `id` is a valid study ID: the ID pattern and 3 to 64 characters.
pub fn is_valid_study_id(id: &str) -> bool {
    (3..=64).contains(&id.len()) && matches_id_pattern(id)
}

/// Whether `id` matches `^q-[0-9a-f]{8}$`.
pub fn is_valid_question_id(id: &str) -> bool {
    id.strip_prefix("q-").is_some_and(|hex| {
        hex.len() == 8
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-76ac5b2e
    fn study_id_validation() {
        assert!(is_valid_study_id("x-vs-docking"));
        assert!(is_valid_study_id("abc"));
        assert!(is_valid_study_id(&"a".repeat(64)));
        assert!(!is_valid_study_id("ab"));
        assert!(!is_valid_study_id(&"a".repeat(65)));
        assert!(!is_valid_study_id("X_vs_Docking"));
        assert!(!is_valid_study_id("a--b"));
        assert!(!is_valid_study_id("-abc"));
        assert!(!is_valid_study_id("abc-"));
    }

    #[test] // rq-a3b4939d
    fn question_id_validation() {
        assert!(is_valid_question_id("q-1a2b3c4d"));
        assert!(!is_valid_question_id("Q1"));
        assert!(!is_valid_question_id("q-1A2B3C4D"));
        assert!(!is_valid_question_id("q-1a2b3c4"));
    }

    #[test] // rq-5d9d0d62
    fn status_round_trip() {
        for s in [
            Status::Draft,
            Status::Preregistered,
            Status::Reported,
            Status::Superseded,
        ] {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        assert_eq!(Status::parse("retrospective"), None);
        assert_eq!(Status::parse("finished"), None);
    }

    #[test] // rq-597a5aff
    fn mode_round_trip_and_predicates() {
        for m in [
            Mode::Confirmatory,
            Mode::Exploratory,
            Mode::Descriptive,
            Mode::Methods,
            Mode::Replication,
            Mode::Retrospective,
        ] {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert!(Mode::Confirmatory.preregisters());
        assert!(Mode::Replication.preregisters());
        assert!(!Mode::Exploratory.preregisters());
        assert!(!Mode::Retrospective.preregisters());
        assert_eq!(
            governed_sections(Mode::Descriptive),
            vec![
                "Question",
                "Deliverables",
                "Scenarios",
                "Variants",
                "Stages",
                "Amendments",
                "Result",
                "Interpretation"
            ]
        );
        assert_eq!(
            governed_sections(Mode::Exploratory),
            vec!["Question", "Variants", "Stages", "Result", "Interpretation"]
        );
    }
}
