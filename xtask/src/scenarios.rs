//! `cargo xtask scenarios`: Gherkin traceability between `rqm/` and the tests.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;

/// The file listing requirements files that are specified but not implemented yet.
pub const PENDING_FILE: &str = "xtask/pending.txt";

/// Directories whose every `rq-` token is a test reference.
const TEST_DIRS: [&str; 3] = ["tests/", "xtask/", "evals/"];

/// The result of the check.
#[derive(Debug, Default)]
pub struct Report {
    /// One line per problem, sorted.
    pub problems: Vec<String>,
    pub referenced: usize,
    pub pending: usize,
}

impl Report {
    pub fn render(&self) -> String {
        let mut out: String = self.problems.iter().map(|p| format!("{p}\n")).collect();
        out.push_str(&format!(
            "{} scenarios referenced, {} pending, {} problem(s)\n",
            self.referenced,
            self.pending,
            self.problems.len()
        ));
        out
    }
}

/// The `rq-<8 lowercase hex>` tokens of a line, as whole words.
fn tokens(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let is_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = line[i..].find("rq-") {
        let start = i + at;
        let end = start + 11;
        let boundary_before = start == 0 || !is_word(bytes[start - 1]);
        let boundary_after = end >= bytes.len() || !is_word(bytes[end]);
        if end <= bytes.len()
            && boundary_before
            && boundary_after
            && bytes[start + 3..end].iter().all(|&b| is_hex(b))
        {
            out.push(&line[start..end]);
        }
        i = start + 3;
    }
    out
}

/// A scenario declared in a requirements file.
struct Scenario {
    id: String,
    file: String,
    title: String,
}

/// Every file below `dir`, as sorted `/`-separated paths relative to `root`, skipping hidden
/// entries and, at the top level, `target/`.
fn walk(root: &Path, dir: &str, out: &mut Vec<String>) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(root.join(dir))?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || (dir.is_empty() && name == "target") {
            continue;
        }
        let rel = if dir.is_empty() {
            name
        } else {
            format!("{dir}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(root, &rel, out)?;
        } else if kind.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}

/// Reads a file as text, or `None` when it is not UTF-8.
fn read_text(root: &Path, rel: &str) -> io::Result<Option<String>> {
    Ok(String::from_utf8(fs::read(root.join(rel))?).ok())
}

/// The paths listed in the pending file.
fn pending_files(root: &Path) -> io::Result<Vec<String>> {
    match fs::read_to_string(root.join(PENDING_FILE)) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

// rq-04882df1
/// Checks scenario traceability of the repository at `root`.
pub fn check(root: &Path) -> io::Result<Report> {
    let mut files = Vec::new();
    walk(root, "", &mut files)?;

    // Declared IDs and scenarios, from the requirements files.
    let mut declared = BTreeSet::new();
    let mut scenarios = Vec::new();
    for file in files
        .iter()
        .filter(|f| f.starts_with("rqm/") && f.ends_with(".md"))
    {
        let Some(text) = read_text(root, file)? else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let found = tokens(line);
            declared.extend(found.iter().map(|t| t.to_string()));
            let trimmed = line.trim_start();
            if !trimmed.starts_with('@') {
                continue;
            }
            let Some(next) = lines[i + 1..].iter().find(|l| !l.trim().is_empty()) else {
                continue;
            };
            let next = next.trim_start();
            let title = next
                .strip_prefix("Scenario Outline:")
                .or_else(|| next.strip_prefix("Scenario:"));
            if let Some(title) = title {
                for id in trimmed
                    .split_whitespace()
                    .filter_map(|t| t.strip_prefix('@'))
                {
                    if tokens(id) == [id] {
                        scenarios.push(Scenario {
                            id: id.to_string(),
                            file: file.clone(),
                            title: title.trim().to_string(),
                        });
                    }
                }
            }
        }
    }

    // References and dangling tokens, from everything outside `rqm/`.
    let mut references: BTreeMap<String, usize> = BTreeMap::new();
    let mut problems = Vec::new();
    for file in files.iter().filter(|f| !f.starts_with("rqm/")) {
        let Some(text) = read_text(root, file)? else {
            continue;
        };
        let test_file = TEST_DIRS.iter().any(|d| file.starts_with(d));
        let source_file = file.starts_with("src/");
        for (n, line) in text.lines().enumerate() {
            let is_test_line = test_file || (source_file && line.contains("#[test]"));
            for id in tokens(line) {
                if !declared.contains(id) {
                    problems.push(format!("dangling {id} {file}:{}", n + 1));
                } else if is_test_line {
                    *references.entry(id.to_string()).or_default() += 1;
                }
            }
        }
    }

    // Pending files.
    let pending = pending_files(root)?;
    let pending_set: BTreeSet<&str> = pending.iter().map(String::as_str).collect();
    for path in &pending {
        if !root.join(path).is_file() {
            problems.push(format!("missing-pending {path}"));
            continue;
        }
        let all_referenced = scenarios
            .iter()
            .filter(|s| &s.file == path)
            .all(|s| references.contains_key(&s.id));
        if all_referenced {
            problems.push(format!("stale-pending {path}"));
        }
    }

    let mut report = Report::default();
    for s in &scenarios {
        if references.contains_key(&s.id) {
            report.referenced += 1;
        } else if pending_set.contains(s.file.as_str()) {
            report.pending += 1;
        } else {
            problems.push(format!("unreferenced {} {}: {}", s.id, s.file, s.title));
        }
    }
    problems.sort();
    problems.dedup();
    report.problems = problems;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An ID built at run time, so this file contains no literal IDs of its own.
    fn id(hex: &str) -> String {
        format!("{}-{hex}", "rq")
    }

    /// A repository with one requirements file `rqm/a.md` holding scenarios `ids`.
    struct Repo {
        root: PathBuf,
    }

    impl Repo {
        fn new(ids: &[String]) -> Repo {
            static N: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "xtask-scenarios-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&root);
            let repo = Repo { root };
            let mut md = format!("# Feature: A <!-- {} -->\n\n```gherkin\n", id("0000aaaa"));
            for (n, i) in ids.iter().enumerate() {
                md.push_str(&format!("  @{i}\n  Scenario: Case {n}\n    Then ok\n\n"));
            }
            md.push_str("```\n");
            repo.write("rqm/a.md", &md);
            repo
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn check(&self) -> Report {
            check(&self.root).unwrap()
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn tokens_are_whole_words() {
        let a = id("1a2b3c4d");
        assert_eq!(
            tokens(&format!("// {a} {a}x x{a} @{a}")),
            vec![a.as_str(), &a]
        );
        assert!(tokens("rq-1A2B3C4D rq-1a2b3c").is_empty());
    }

    #[test] // rq-6602ff7a rq-04882df1
    fn unreferenced_scenarios_are_found() {
        let (a, b) = (id("1111aaaa"), id("2222bbbb"));
        let repo = Repo::new(&[a.clone(), b.clone()]);
        repo.write("tests/t.rs", &format!("#[test] // {a}\nfn t() {{}}\n"));
        let report = repo.check();
        assert_eq!(
            report.problems,
            vec![format!("unreferenced {b} rqm/a.md: Case 1")]
        );
        assert_eq!(report.referenced, 1);
        assert!(report
            .render()
            .ends_with("1 scenarios referenced, 0 pending, 1 problem(s)\n"));
    }

    #[test] // rq-59f0f637 rq-04882df1
    fn dangling_scenario_references_are_found() {
        let a = id("1111aaaa");
        let repo = Repo::new(std::slice::from_ref(&a));
        let ghost = id("deadbeef");
        repo.write(
            "tests/t.rs",
            &format!("// {a}\n\n#[test] // {ghost}\nfn t() {{}}\n"),
        );
        repo.write("README.md", &format!("see {ghost}\n"));
        let report = repo.check();
        assert_eq!(
            report.problems,
            vec![
                format!("dangling {ghost} README.md:1"),
                format!("dangling {ghost} tests/t.rs:3"),
            ]
        );
    }

    #[test] // rq-cc74f676 rq-04882df1
    fn scenarios_of_pending_files_need_no_reference() {
        let repo = Repo::new(&[id("1111aaaa"), id("2222bbbb")]);
        repo.write(PENDING_FILE, "# planned\n\nrqm/a.md\n");
        let report = repo.check();
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!((report.referenced, report.pending), (0, 2));
    }

    #[test] // rq-aae2e08f rq-04882df1
    fn a_fully_referenced_pending_file_is_stale() {
        let a = id("1111aaaa");
        let repo = Repo::new(std::slice::from_ref(&a));
        repo.write("tests/t.rs", &format!("// {a}\n"));
        repo.write(PENDING_FILE, "rqm/a.md\n");
        assert_eq!(repo.check().problems, vec!["stale-pending rqm/a.md"]);
    }

    #[test] // rq-b1aa1cec rq-04882df1
    fn a_pending_entry_must_exist() {
        let a = id("1111aaaa");
        let repo = Repo::new(std::slice::from_ref(&a));
        repo.write("tests/t.rs", &format!("// {a}\n"));
        repo.write(PENDING_FILE, "rqm/nonexistent.md\n");
        assert_eq!(
            repo.check().problems,
            vec!["missing-pending rqm/nonexistent.md"]
        );
    }

    #[test] // rq-d7684c14 rq-04882df1
    fn only_tests_count_as_references() {
        let (a, b) = (id("1111aaaa"), id("2222bbbb"));
        let repo = Repo::new(&[a.clone(), b.clone()]);
        repo.write(
            "src/lib.rs",
            &format!("// {a}\nfn f() {{}}\n\n#[test] // {b}\nfn t() {{}}\n"),
        );
        assert_eq!(
            repo.check().problems,
            vec![format!("unreferenced {a} rqm/a.md: Case 0")]
        );
    }

    #[test]
    fn hidden_directories_and_target_are_skipped() {
        let a = id("1111aaaa");
        let repo = Repo::new(std::slice::from_ref(&a));
        repo.write("tests/t.rs", &format!("// {a}\n"));
        let ghost = id("deadbeef");
        repo.write(".riprap/skill.md", &format!("example {ghost}\n"));
        repo.write("target/debug/x.rs", &format!("{ghost}\n"));
        assert!(repo.check().problems.is_empty());
    }
}
