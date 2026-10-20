//! The frozen fingerprint of a study.

use sha2::{Digest, Sha256};

use super::{Study, FROZEN_SECTIONS};

/// Strips trailing whitespace from each line and removes leading and trailing blank lines.
fn normalize(content: &[(u32, String)]) -> Vec<&str> {
    let lines: Vec<&str> = content.iter().map(|(_, l)| l.trim_end()).collect();
    let start = lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(start, |i| i + 1);
    lines[start..end].to_vec()
}

// rq-f5a0149d rq-08c7718c
/// Computes the frozen fingerprint: SHA-256 over the study ID and the normalized content of the
/// first occurrence of each frozen section, in fixed order. A missing section contributes only its
/// heading line.
pub fn frozen_fingerprint(study: &Study) -> String {
    let mut text = format!("study:{}\n", study.id);
    for name in FROZEN_SECTIONS {
        text.push_str(&format!("## {name}\n"));
        if let Some(section) = study.section(name) {
            for line in normalize(&section.content) {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Whether `s` has the fingerprint format: `sha256:` and 64 lowercase hex characters.
pub fn is_fingerprint_format(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::study::parse_study_text;

    const FRONT: &str = "---\nstudy: abc\n---\n";

    fn fp(id: &str, body: &str) -> String {
        frozen_fingerprint(&parse_study_text(id, "STUDY.md", &format!("{FRONT}{body}")).unwrap())
    }

    #[test] // rq-08c7718c rq-f5a0149d
    fn matches_the_specified_construction() {
        let body = "## Question\n\nWhy?  \n\n## Primary endpoint\nrho\n## Decision rule\n\
                    ## Scenarios\n```gherkin\n@S1\nScenario: x\n```\n";
        let expected_input = "study:abc\n## Question\nWhy?\n## Primary endpoint\nrho\n\
                              ## Decision rule\n## Scenarios\n```gherkin\n@S1\nScenario: x\n```\n";
        let digest = Sha256::digest(expected_input.as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(fp("abc", body), format!("sha256:{hex}"));
        assert!(is_fingerprint_format(&fp("abc", body)));
    }

    #[test] // rq-08c7718c
    fn ignores_whitespace_order_and_other_sections() {
        let a = fp(
            "abc",
            "## Question\nWhy?\n## Scenarios\nS\n## Stages\none\n",
        );
        let b = fp(
            "abc",
            "## Scenarios\r\n\r\nS   \r\n## Question\r\nWhy?\t\r\n## Stages\r\ntwo\r\n",
        );
        assert_eq!(a, b);
        assert_ne!(a, fp("abd", "## Question\nWhy?\n## Scenarios\nS\n"));
        assert_ne!(a, fp("abc", "## Question\nWhy not?\n## Scenarios\nS\n"));
    }

    #[test]
    fn fingerprint_format() {
        assert!(!is_fingerprint_format("sha256:ABC"));
        assert!(!is_fingerprint_format(&format!("md5:{}", "a".repeat(64))));
    }
}
