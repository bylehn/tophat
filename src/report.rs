//! Rendering of check results in the human-readable and JSON formats.

use crate::check::{Diagnostic, Severity};
use serde_json::{Map, Value, json};

/// Version of the JSON output structure; changes only on incompatible changes.
pub const JSON_FORMAT_VERSION: u32 = 1;

/// Counts of diagnostics per severity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub legacy: usize,
}

impl Summary {
    pub fn of(diagnostics: &[Diagnostic]) -> Self {
        let count = |s: Severity| diagnostics.iter().filter(|d| d.severity == s).count();
        Summary {
            errors: count(Severity::Error),
            warnings: count(Severity::Warning),
            legacy: count(Severity::Legacy),
        }
    }
}

fn render_line(out: &mut String, d: &Diagnostic) {
    out.push_str(&d.path);
    if let Some(line) = d.line {
        out.push_str(&format!(":{line}"));
    }
    out.push_str(&format!(": {}[{}]: {}\n", d.severity, d.rule, d.message));
    if let Some(hint) = &d.hint {
        out.push_str(&format!("    hint: {hint}\n"));
    }
}

// rq-b757d2fc
/// Renders diagnostics (already sorted) in the human-readable format: errors and warnings grouped
/// by path, then a `Legacy` section, then a summary line.
pub fn render_human(diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    let mut previous_path: Option<&str> = None;
    for d in diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Legacy)
    {
        if previous_path.is_some_and(|p| p != d.path) {
            out.push('\n');
        }
        previous_path = Some(&d.path);
        render_line(&mut out, d);
    }

    let mut legacy = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Legacy)
        .peekable();
    if legacy.peek().is_some() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Legacy\n");
        for d in legacy {
            render_line(&mut out, d);
        }
    }

    let s = Summary::of(diagnostics);
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!(
        "{} error(s), {} warning(s), {} legacy item(s)\n",
        s.errors, s.warnings, s.legacy
    ));
    out
}

// rq-b757d2fc
/// Renders diagnostics (already sorted) as the JSON object specified for `--json`.
pub fn render_json(diagnostics: &[Diagnostic]) -> String {
    let items: Vec<Value> = diagnostics
        .iter()
        .map(|d| {
            let mut obj = Map::new();
            obj.insert("rule".into(), json!(d.rule));
            obj.insert("severity".into(), json!(d.severity.as_str()));
            obj.insert("path".into(), json!(d.path));
            if let Some(line) = d.line {
                obj.insert("line".into(), json!(line));
            }
            obj.insert("message".into(), json!(d.message));
            if let Some(hint) = &d.hint {
                obj.insert("hint".into(), json!(hint));
            }
            Value::Object(obj)
        })
        .collect();
    let s = Summary::of(diagnostics);
    let doc = json!({
        "format_version": JSON_FORMAT_VERSION,
        "tophat_version": env!("CARGO_PKG_VERSION"),
        "diagnostics": items,
        "summary": { "errors": s.errors, "warnings": s.warnings, "legacy": s.legacy },
    });
    let mut text = serde_json::to_string_pretty(&doc).expect("JSON values always serialize");
    text.push('\n');
    text
}
