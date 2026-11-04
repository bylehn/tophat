//! The question tree in `PROJECT.md`: parsing, the `question` check rule family, and the
//! `tophat question` commands. Specified in `rqm/question-tree.md`.

mod commands;
mod rules;

use std::fmt;

use crate::markdown::{FenceState, FenceTracker, atx_heading, numbered_lines};

pub use commands::{StampOutcome, render_tree, render_tree_json, stamp, stamp_outcome};
pub use rules::question_rules;

/// File name of the project document, at the project root.
pub const PROJECT_FILE: &str = "PROJECT.md";

/// The separator between a question's ID and its text: a middle dot surrounded by spaces.
pub const SEPARATOR: char = '·';

// rq-f448ed00
/// Status of a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionStatus {
    Open,
    Answered,
    Dropped,
}

impl QuestionStatus {
    pub fn parse(s: &str) -> Option<QuestionStatus> {
        Some(match s {
            "open" => QuestionStatus::Open,
            "answered" => QuestionStatus::Answered,
            "dropped" => QuestionStatus::Dropped,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            QuestionStatus::Open => "open",
            QuestionStatus::Answered => "answered",
            QuestionStatus::Dropped => "dropped",
        }
    }
}

// rq-e956ef6a
/// A question parsed from `PROJECT.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// `None` when the heading is unstamped or its ID is malformed.
    pub id: Option<String>,
    pub text: String,
    /// Heading level, 2 to 6.
    pub level: u8,
    pub heading_line: u32,
    /// Index of the parent question in [`QuestionTree::questions`].
    pub parent: Option<usize>,
    /// Raw status value and its line, when the body starts with a status line.
    pub status: Option<(String, u32)>,
    pub answer: Option<(String, u32)>,
    pub reason: Option<(String, u32)>,
    /// Citations (trimmed, empty items dropped) and the evidence line.
    pub evidence: Option<(Vec<String>, u32)>,
}

impl Question {
    /// The parsed status, if the status line is present and valid.
    pub fn parsed_status(&self) -> Option<QuestionStatus> {
        self.status
            .as_ref()
            .and_then(|(s, _)| QuestionStatus::parse(s))
    }
}

/// A structural problem with a question heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadingIssue {
    Unstamped { line: u32 },
    MalformedId { line: u32, token: String },
    TextEmpty { line: u32 },
    LevelSkip { line: u32 },
}

// rq-dcafcef0
/// The questions of `PROJECT.md` in file order, and the structural problems found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuestionTree {
    pub questions: Vec<Question>,
    pub issues: Vec<HeadingIssue>,
}

impl QuestionTree {
    /// The index of the first question with this ID.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.questions
            .iter()
            .position(|q| q.id.as_deref() == Some(id))
    }

    /// Whether question `candidate` is a strict descendant of question `ancestor`.
    pub fn is_descendant(&self, candidate: usize, ancestor: usize) -> bool {
        let mut current = self.questions[candidate].parent;
        while let Some(i) = current {
            if i == ancestor {
                return true;
            }
            current = self.questions[i].parent;
        }
        false
    }
}

// rq-7bde5706
/// Error returned by the `tophat question` commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionError {
    /// `PROJECT.md` does not exist.
    ProjectFileMissing,
    /// `stamp` left these malformed or duplicated IDs unchanged.
    Unresolved(Vec<String>),
    Io(String),
}

impl fmt::Display for QuestionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuestionError::ProjectFileMissing => {
                write!(f, "{PROJECT_FILE} does not exist at the project root")
            }
            QuestionError::Unresolved(ids) => write!(
                f,
                "left unchanged, fix by hand: {} (malformed or duplicated question IDs)",
                ids.join(", ")
            ),
            QuestionError::Io(message) => write!(f, "I/O error: {message}"),
        }
    }
}

impl std::error::Error for QuestionError {}

impl From<std::io::Error> for QuestionError {
    fn from(e: std::io::Error) -> Self {
        QuestionError::Io(e.to_string())
    }
}

/// Whether `s` is `q-` followed by 8 lowercase hex characters.
pub fn is_valid_question_id(s: &str) -> bool {
    crate::study::is_valid_question_id(s)
}

/// How a question heading's text identifies the question.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HeadingId<'a> {
    /// `q-<valid id> · <text>`.
    Stamped { id: &'a str, text: &'a str },
    /// `q-<token> · <text>` where the token is not a valid ID.
    Malformed { token: &'a str, text: &'a str },
    /// No ID prefix.
    Unstamped,
}

// rq-8148f10b
/// Classifies the text of a question heading.
pub(crate) fn heading_id(text: &str) -> HeadingId<'_> {
    if !text.starts_with("q-") {
        return HeadingId::Unstamped;
    }
    let (token, rest) = text.split_once(' ').unwrap_or((text, ""));
    let Some(after) = rest.trim_start().strip_prefix(SEPARATOR) else {
        return HeadingId::Unstamped;
    };
    let question = after.trim();
    if is_valid_question_id(token) {
        HeadingId::Stamped {
            id: token,
            text: question,
        }
    } else {
        HeadingId::Malformed {
            token,
            text: question,
        }
    }
}

/// Extracts the value after `prefix` on a line that starts with it, trimmed.
fn line_value<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.trim_start().strip_prefix(prefix).map(str::trim)
}

// rq-0e66003f rq-ba118d75 rq-44727473 rq-76dbbbfa
/// Parses `PROJECT.md` text into a question tree. Never fails; structural problems are recorded
/// in [`QuestionTree::issues`].
pub fn parse_project(text: &str) -> QuestionTree {
    let mut tree = QuestionTree::default();
    let mut fences = FenceTracker::default();
    // Ancestors of the current position: (level, question index).
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut previous_level: Option<u8> = None;
    // Whether the current question's status line position has passed (first non-blank line seen).
    let mut seen_first_body_line = false;
    // The question whose body we are in; `None` before the first question or after a `#` heading.
    let mut current: Option<usize> = None;

    for (no, line) in numbered_lines(text) {
        let state = fences.next(line);
        if state == FenceState::Outside
            && let Some((level, heading_text)) = atx_heading(line)
        {
            if level == 1 {
                current = None;
                continue;
            }
            let skipped = match previous_level {
                None => level != 2,
                Some(prev) => level > prev + 1,
            };
            if skipped {
                tree.issues.push(HeadingIssue::LevelSkip { line: no });
            }
            previous_level = Some(level);
            while stack.last().is_some_and(|&(l, _)| l >= level) {
                stack.pop();
            }
            let parent = stack.last().map(|&(_, i)| i);
            let (id, question_text) = match heading_id(heading_text) {
                HeadingId::Stamped { id, text } => {
                    if text.is_empty() {
                        tree.issues.push(HeadingIssue::TextEmpty { line: no });
                    }
                    (Some(id.to_string()), text.to_string())
                }
                HeadingId::Malformed { token, text } => {
                    tree.issues.push(HeadingIssue::MalformedId {
                        line: no,
                        token: token.to_string(),
                    });
                    (None, text.to_string())
                }
                HeadingId::Unstamped => {
                    tree.issues.push(HeadingIssue::Unstamped { line: no });
                    (None, heading_text.to_string())
                }
            };
            let index = tree.questions.len();
            tree.questions.push(Question {
                id,
                text: question_text,
                level,
                heading_line: no,
                parent,
                status: None,
                answer: None,
                reason: None,
                evidence: None,
            });
            stack.push((level, index));
            current = Some(index);
            seen_first_body_line = false;
            continue;
        }

        let Some(index) = current else { continue };
        let question = &mut tree.questions[index];
        if !seen_first_body_line {
            if line.trim().is_empty() {
                continue;
            }
            seen_first_body_line = true;
            if let Some(value) =
                line_value(line, "status:").filter(|_| state == FenceState::Outside)
            {
                question.status = Some((value.to_string(), no));
                continue;
            }
        }
        if state != FenceState::Outside {
            continue;
        }
        if question.answer.is_none()
            && let Some(value) = line_value(line, "**Answer:**")
        {
            question.answer = Some((value.to_string(), no));
            continue;
        }
        if question.reason.is_none()
            && let Some(value) = line_value(line, "**Reason:**")
        {
            question.reason = Some((value.to_string(), no));
            continue;
        }
        if question.evidence.is_none()
            && let Some(value) = line_value(line, "Evidence:")
        {
            let citations = value
                .split(',')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string)
                .collect();
            question.evidence = Some((citations, no));
        }
    }
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "# Method X\n\nPreamble.\n\n\
        ## q-1a2b3c4d · Can X improve?\nstatus: open\n\n\
        ### q-5e6f7a8b · Is X accurate?\nstatus: answered\n\n**Answer:** Yes.\nEvidence: x-energy, q-00000000 ,\n\n\
        ### q-9c0d1e2f · Does X beat docking?\nstatus: open\n\n\
        #### q-3b4c5d6e · Client targets?\nstatus: dropped\n\n**Reason:** Withdrawn.\n";

    #[test] // rq-0e66003f rq-8148f10b rq-76dbbbfa
    fn parses_the_example() {
        let tree = parse_project(EXAMPLE);
        assert!(tree.issues.is_empty(), "{:?}", tree.issues);
        let ids: Vec<_> = tree
            .questions
            .iter()
            .map(|q| q.id.as_deref().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["q-1a2b3c4d", "q-5e6f7a8b", "q-9c0d1e2f", "q-3b4c5d6e"]
        );
        assert_eq!(
            tree.questions[1].parsed_status(),
            Some(QuestionStatus::Answered)
        );
        assert_eq!(tree.questions[1].answer.as_ref().unwrap().0, "Yes.");
        assert_eq!(
            tree.questions[1].evidence.as_ref().unwrap().0,
            vec!["x-energy".to_string(), "q-00000000".to_string()]
        );
        assert_eq!(tree.questions[3].reason.as_ref().unwrap().0, "Withdrawn.");
        assert_eq!(tree.questions[3].parent, Some(2));
        assert_eq!(tree.questions[0].text, "Can X improve?");
        assert!(tree.is_descendant(3, 0));
        assert!(!tree.is_descendant(1, 2));
    }

    #[test] // rq-8148f10b
    fn classifies_heading_ids() {
        assert_eq!(
            heading_id("q-1a2b3c4d · A?"),
            HeadingId::Stamped {
                id: "q-1a2b3c4d",
                text: "A?"
            }
        );
        assert_eq!(
            heading_id("q-12 · A?"),
            HeadingId::Malformed {
                token: "q-12",
                text: "A?"
            }
        );
        assert_eq!(heading_id("q-learning works?"), HeadingId::Unstamped);
        assert_eq!(heading_id("Can X?"), HeadingId::Unstamped);
        assert_eq!(
            heading_id("q-1a2b3c4d ·"),
            HeadingId::Stamped {
                id: "q-1a2b3c4d",
                text: ""
            }
        );
    }

    #[test] // rq-76dbbbfa
    fn status_must_be_the_first_non_blank_line() {
        let tree = parse_project("## q-1a2b3c4d · A?\n\nDescription.\nstatus: open\n");
        assert_eq!(tree.questions[0].status, None);
        let tree = parse_project("## q-1a2b3c4d · A?\n\n  status: done\n");
        assert_eq!(tree.questions[0].status, Some(("done".into(), 3)));
    }
}
