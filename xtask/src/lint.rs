//! `cargo xtask lint-skills`: the skill lint rules of `rqm/template/skills.md`, applied to a
//! rendered project.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// Where the canonical skills live.
const CANONICAL: &str = ".tophat/managed/skills";

/// Each agent and the directory of its skill adapters.
const ADAPTER_DIRS: [(&str, &str); 3] = [
    ("claude-code", ".claude/skills"),
    ("codex", ".agents/skills"),
    ("opencode", ".opencode/skills"),
];

/// Adapters may hold at most this many lines after their front matter.
const ADAPTER_MAX_LINES: usize = 15;

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub path: String,
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{} {}:{} {}", self.rule, self.path, line, self.message),
            None => write!(f, "{} {} {}", self.rule, self.path, self.message),
        }
    }
}

/// A skill file split into its front matter fields and body.
struct SkillFile {
    name: Option<String>,
    description: Option<String>,
    /// Lines after the front matter, with their 1-based line numbers.
    body: Vec<(usize, String)>,
}

fn parse(text: &str) -> SkillFile {
    let lines: Vec<&str> = text.lines().collect();
    let mut file = SkillFile {
        name: None,
        description: None,
        body: Vec::new(),
    };
    let end = if lines.first() == Some(&"---") {
        lines
            .iter()
            .skip(1)
            .position(|l| *l == "---")
            .map(|i| i + 1)
    } else {
        None
    };
    if let Some(end) = end {
        for line in &lines[1..end] {
            let field = |key: &str| {
                line.strip_prefix(key)
                    .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|v| !v.is_empty())
            };
            file.name = file.name.take().or_else(|| field("name:"));
            file.description = file.description.take().or_else(|| field("description:"));
        }
    }
    let start = end.map_or(0, |e| e + 1);
    file.body = lines
        .iter()
        .enumerate()
        .skip(start)
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect();
    file
}

/// The agents selected in the project's `.copier-answers.yml`.
fn selected_agents(project: &Path) -> io::Result<BTreeSet<String>> {
    let text = fs::read_to_string(project.join(".copier-answers.yml"))?;
    let mut agents = BTreeSet::new();
    let mut in_agents = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("agents:") {
            let rest = rest.trim();
            if let Some(inline) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                agents.extend(
                    inline
                        .split(',')
                        .map(|a| a.trim().trim_matches(['"', '\'']).to_string())
                        .filter(|a| !a.is_empty()),
                );
            } else {
                in_agents = true;
            }
            continue;
        }
        if in_agents {
            match line.strip_prefix("- ") {
                Some(agent) => {
                    agents.insert(agent.trim().trim_matches(['"', '\'']).to_string());
                }
                None => in_agents = false,
            }
        }
    }
    Ok(agents)
}

/// The command and subcommands of a `tophat --help` usage text.
fn usage_commands(usage: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in usage.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let words: Vec<&str> = rest.split_whitespace().collect();
        let is_word =
            |w: &str| !w.starts_with('-') && w.chars().all(|c| c.is_ascii_lowercase() || c == '-');
        let Some(command) = words.first().copied().filter(|w| is_word(w)) else {
            continue;
        };
        out.insert(command.to_string());
        if let Some(sub) = words.get(1).copied().filter(|w| is_word(w)) {
            out.insert(format!("{command} {sub}"));
        }
    }
    out
}

/// Whether the character before an occurrence allows it to start a word or an invocation.
fn starts_word(line: &str, at: usize) -> bool {
    line[..at]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || "`(\"'[".contains(c))
}

/// The `tophat ...` mentions inside code (inline code spans or fenced blocks) of a line.
fn tophat_mentions(line: &str, in_fence: bool) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in line.match_indices("tophat ") {
        let inside_code = in_fence || line[..at].matches('`').count() % 2 == 1;
        if !inside_code || !starts_word(line, at) {
            continue;
        }
        let rest = &line[at + "tophat ".len()..];
        let rest = rest.split('`').next().unwrap_or(rest);
        out.push(format!("tophat {}", rest.trim()));
    }
    out
}

/// Whether a mention names a known command (and subcommand, when the command has any).
fn known_mention(mention: &str, commands: &BTreeSet<String>) -> Result<(), String> {
    let words: Vec<&str> = mention.split_whitespace().skip(1).collect();
    let Some(command) = words.first() else {
        return Err("tophat".into());
    };
    let command = command.trim_end_matches(['.', ',', ';', ':', ')']);
    if !commands.contains(command) {
        return Err(format!("tophat {command}"));
    }
    let has_subcommands = commands
        .iter()
        .any(|c| c.starts_with(&format!("{command} ")));
    if !has_subcommands {
        return Ok(());
    }
    match words.get(1) {
        Some(sub) if sub.chars().next().is_some_and(|c| c.is_ascii_lowercase()) => {
            let sub = sub.trim_end_matches(['.', ',', ';', ':', ')']);
            if commands.contains(&format!("{command} {sub}")) {
                Ok(())
            } else {
                Err(format!("tophat {command} {sub}"))
            }
        }
        _ => Ok(()),
    }
}

/// Agent-specific wording in a canonical skill line, if any.
fn agent_specific(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for name in ["claude", "codex", "opencode"] {
        for (at, _) in lower.match_indices(name) {
            let before = lower[..at].chars().next_back();
            let after = lower[at + name.len()..].chars().next();
            let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
            if boundary(before) && boundary(after) {
                return Some(format!("mentions `{}`", &line[at..at + name.len()]));
            }
        }
    }
    for marker in ["/th-", "$th-"] {
        for (at, _) in line.match_indices(marker) {
            if starts_word(line, at) {
                return Some(format!("invokes a skill as `{}…`", marker));
            }
        }
    }
    None
}

// rq-4a032ea7
/// Applies the skill lint rules to the project at `project`, with `usage` the output of
/// `tophat --help`. Findings are sorted.
pub fn lint_skills(project: &Path, usage: &str) -> io::Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let mut finding = |rule, path: &str, line, message: String| {
        findings.push(Finding {
            rule,
            path: path.to_string(),
            line,
            message,
        })
    };
    let commands = usage_commands(usage);

    let mut skills: Vec<String> = match fs::read_dir(project.join(CANONICAL)) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    skills.sort();
    let mut descriptions = std::collections::BTreeMap::new();

    for skill in &skills {
        let rel = format!("{CANONICAL}/{skill}/SKILL.md");
        let Ok(text) = fs::read_to_string(project.join(&rel)) else {
            finding("skill.front-matter", &rel, None, "is missing".into());
            continue;
        };
        let file = parse(&text);
        match (&file.name, &file.description) {
            (Some(name), Some(_)) if name == skill => {}
            (Some(name), Some(_)) => finding(
                "skill.front-matter",
                &rel,
                Some(2),
                format!("`name: {name}` differs from its directory `{skill}`"),
            ),
            _ => finding(
                "skill.front-matter",
                &rel,
                Some(1),
                "needs `name` and `description` in its front matter".into(),
            ),
        }
        if let Some(d) = &file.description {
            descriptions.insert(skill.clone(), d.clone());
        }
        let marked = file
            .body
            .iter()
            .filter(|(_, l)| !l.trim().is_empty())
            .take(5)
            .any(|(_, l)| l.contains("TopHat-managed"))
            && file.body.iter().take(6).any(|(_, l)| l.contains("<!--"));
        if !marked {
            finding(
                "skill.marker",
                &rel,
                None,
                "needs the TopHat-managed marker in an HTML comment within five lines of its front matter".into(),
            );
        }

        let mut in_fence = false;
        for (n, line) in &file.body {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if let Some(what) = agent_specific(line) {
                finding("skill.agent-specific", &rel, Some(*n), what);
            }
            for mention in tophat_mentions(line, in_fence) {
                if let Err(unknown) = known_mention(&mention, &commands) {
                    finding(
                        "skill.cli-unknown",
                        &rel,
                        Some(*n),
                        format!("`{unknown}` is not a tophat command"),
                    );
                }
            }
        }

        let local = format!(".tophat/user/skills/{skill}/local.md");
        let last_section = file
            .body
            .iter()
            .rposition(|(_, l)| l.starts_with("## "))
            .map(|i| &file.body[i..]);
        let defers = last_section.is_some_and(|section| {
            section[0].1.trim() == "## Project Extensions"
                && section.iter().any(|(_, l)| l.contains(&local))
        });
        if !defers {
            finding(
                "skill.local-missing",
                &rel,
                None,
                format!("must end with a `## Project Extensions` section naming `{local}`"),
            );
        }
        if !project.join(&local).is_file() {
            finding("skill.local-missing", &local, None, "does not exist".into());
        }
    }

    let selected = selected_agents(project)?;
    for (agent, dir) in ADAPTER_DIRS {
        let present: BTreeSet<String> = fs::read_dir(project.join(dir))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.path().join("SKILL.md").is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        if !selected.contains(agent) {
            for skill in &present {
                finding(
                    "skill.adapter-missing",
                    &format!("{dir}/{skill}/SKILL.md"),
                    None,
                    format!("is an adapter for `{agent}`, which the project did not select"),
                );
            }
            continue;
        }
        for skill in &skills {
            let rel = format!("{dir}/{skill}/SKILL.md");
            if !present.contains(skill) {
                finding(
                    "skill.adapter-missing",
                    &rel,
                    None,
                    format!("is missing: `{agent}` is selected"),
                );
                continue;
            }
            let file = parse(&fs::read_to_string(project.join(&rel))?);
            if file.name.as_deref() != Some(skill.as_str()) || file.description.is_none() {
                finding(
                    "skill.front-matter",
                    &rel,
                    Some(1),
                    format!("needs `name: {skill}` and a `description`"),
                );
            }
            if file.description.is_some() && file.description != descriptions.get(skill).cloned() {
                finding(
                    "skill.adapter-content",
                    &rel,
                    None,
                    "its description differs from the canonical skill's".into(),
                );
            }
            if !file.body.iter().any(|(_, l)| l.contains("TopHat-managed")) {
                finding(
                    "skill.marker",
                    &rel,
                    None,
                    "needs the TopHat-managed marker".into(),
                );
            }
            let pointer = format!("Read `{CANONICAL}/{skill}/SKILL.md` completely");
            if !file.body.iter().any(|(_, l)| l.contains(&pointer)) {
                finding(
                    "skill.adapter-content",
                    &rel,
                    None,
                    format!("does not point at its canonical skill (`{pointer}`)"),
                );
            }
            let lines = file
                .body
                .iter()
                .rposition(|(_, l)| !l.trim().is_empty())
                .map_or(0, |i| i + 1);
            if lines > ADAPTER_MAX_LINES {
                finding(
                    "skill.adapter-content",
                    &rel,
                    None,
                    format!(
                        "has {lines} lines after its front matter; at most {ADAPTER_MAX_LINES}"
                    ),
                );
            }
        }
    }

    findings.sort_by(|a, b| (a.rule, &a.path, a.line).cmp(&(b.rule, &b.path, b.line)));
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE: &str = "Usage: tophat <command>\n\nCommands:\n  status [<STUDY>...] [--json]   Show\n  study new <ID> --question <Q>  Create\n                                 Continued description\n  check [--json]                 Check\n\nOptions:\n  -h, --help  Help\n";

    #[test]
    fn usage_is_read_into_commands() {
        let c = usage_commands(USAGE);
        assert!(c.contains("status") && c.contains("study new") && c.contains("check"));
        assert!(
            !c.iter()
                .any(|x| x.starts_with('-') || x.contains("Continued"))
        );
    }

    #[test]
    fn mentions_are_checked_inside_code_only() {
        let c = usage_commands(USAGE);
        assert_eq!(
            tophat_mentions("the tophat binary", false),
            Vec::<String>::new()
        );
        let m = tophat_mentions("run `tophat study new x` and `tophat check`", false);
        assert_eq!(m, vec!["tophat study new x", "tophat check"]);
        assert!(m.iter().all(|x| known_mention(x, &c).is_ok()));
        assert_eq!(
            known_mention("tophat study copy x", &c),
            Err("tophat study copy".into())
        );
        assert_eq!(
            known_mention("tophat frobnicate", &c),
            Err("tophat frobnicate".into())
        );
        assert!(known_mention("tophat study <id>", &c).is_ok());
    }

    #[test]
    fn agent_wording_is_found() {
        assert!(agent_specific("use the Claude question tool").is_some());
        assert!(agent_specific("invoke `/th-run` next").is_some());
        assert!(agent_specific("see .tophat/managed/skills/th-run/SKILL.md").is_none());
        assert!(agent_specific("use th-run next").is_none());
        assert!(agent_specific("a preclaude word").is_none());
    }
}
