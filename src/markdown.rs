//! Line-level Markdown handling shared by `STUDY.md` and `SUMMARY.md`: front-matter splitting,
//! fenced code blocks, `##` sections, and HTML comments.

/// A line of the source with its 1-based line number. Line endings (`\n` or `\r\n`) are removed.
pub type NumberedLine<'a> = (u32, &'a str);

/// Splits text into numbered lines without their line endings. A trailing line ending does not
/// produce an extra empty line.
pub fn numbered_lines(text: &str) -> Vec<NumberedLine<'_>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines: Vec<NumberedLine<'_>> = text
        .split('\n')
        .enumerate()
        .map(|(i, l)| (i as u32 + 1, l.strip_suffix('\r').unwrap_or(l)))
        .collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// The result of splitting a document into YAML front matter and a Markdown body.
#[derive(Debug)]
pub struct Split<'a> {
    /// The front-matter lines between the `---` delimiters.
    pub front: Vec<NumberedLine<'a>>,
    /// The body lines after the closing `---`.
    pub body: Vec<NumberedLine<'a>>,
}

/// Splits front matter delimited by `---` lines from the body.
pub fn split_front_matter<'a>(lines: &[NumberedLine<'a>]) -> Result<Split<'a>, String> {
    let is_delimiter = |l: &str| l.trim_end() == "---";
    match lines.first() {
        Some((_, first)) if is_delimiter(first) => {}
        _ => return Err("the file does not start with a `---` front-matter delimiter".into()),
    }
    let close = lines[1..]
        .iter()
        .position(|(_, l)| is_delimiter(l))
        .map(|i| i + 1)
        .ok_or("the front matter has no closing `---` delimiter")?;
    Ok(Split {
        front: lines[1..close].to_vec(),
        body: lines[close + 1..].to_vec(),
    })
}

/// An open fenced code block.
#[derive(Debug, Clone, Copy)]
struct Fence {
    ch: u8,
    len: usize,
}

/// Strips up to three leading spaces, as CommonMark allows before fences and headings.
fn strip_indent(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|&b| b == b' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}

/// If `line` opens a fenced code block, returns the fence and its info string.
fn fence_open(line: &str) -> Option<(Fence, &str)> {
    let rest = strip_indent(line)?;
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|&b| b == ch).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    if ch == b'`' && info.contains('`') {
        return None;
    }
    Some((Fence { ch, len }, info))
}

/// Whether `line` closes the given fence.
fn fence_closes(line: &str, fence: Fence) -> bool {
    let Some(rest) = strip_indent(line) else {
        return false;
    };
    let len = rest.bytes().take_while(|&b| b == fence.ch).count();
    len >= fence.len && rest[len..].trim().is_empty()
}

/// Tracks whether successive lines are inside a fenced code block.
#[derive(Debug, Default)]
pub struct FenceTracker {
    open: Option<Fence>,
}

/// How a line relates to fenced code blocks.
#[derive(Debug, PartialEq, Eq)]
pub enum FenceState<'a> {
    /// An ordinary line outside any fence.
    Outside,
    /// A line opening a fence, with its info string.
    Opens(&'a str),
    /// A line inside a fence.
    Inside,
    /// A line closing a fence.
    Closes,
}

impl FenceTracker {
    /// Classifies the next line and updates the state.
    pub fn next<'a>(&mut self, line: &'a str) -> FenceState<'a> {
        match self.open {
            Some(fence) if fence_closes(line, fence) => {
                self.open = None;
                FenceState::Closes
            }
            Some(_) => FenceState::Inside,
            None => match fence_open(line) {
                Some((fence, info)) => {
                    self.open = Some(fence);
                    FenceState::Opens(info)
                }
                None => FenceState::Outside,
            },
        }
    }
}

/// If `line` is an ATX heading (`#` to `######` followed by a space, tab or the end of the line),
/// returns its level and trimmed text.
pub fn atx_heading(line: &str) -> Option<(u8, &str)> {
    let rest = strip_indent(line)?;
    let level = rest.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let after = &rest[level..];
    match after.chars().next() {
        None => Some((level as u8, "")),
        Some(' ' | '\t') => Some((level as u8, after.trim())),
        Some(_) => None,
    }
}

/// If `line` is a `##` heading (not `###` or deeper), returns its trimmed name.
pub fn h2_name(line: &str) -> Option<&str> {
    let rest = strip_indent(line)?.strip_prefix("##")?;
    match rest.chars().next() {
        None => Some(""),
        Some(' ' | '\t') => Some(rest.trim()),
        Some(_) => None,
    }
}

/// A `##` section of a Markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading text after `## `, trimmed.
    pub name: String,
    /// 1-based line number of the heading.
    pub heading_line: u32,
    /// The lines after the heading up to the next `##` heading outside a fence, or the end.
    pub content: Vec<(u32, String)>,
}

/// Splits a body into its `##` sections. Lines before the first section are ignored; headings
/// inside fenced code blocks are not section boundaries.
pub fn sections(body: &[NumberedLine<'_>]) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut fences = FenceTracker::default();
    for &(no, line) in body {
        let state = fences.next(line);
        if state == FenceState::Outside
            && let Some(name) = h2_name(line)
        {
            out.push(Section {
                name: name.to_string(),
                heading_line: no,
                content: Vec::new(),
            });
            continue;
        }
        if let Some(section) = out.last_mut() {
            section.content.push((no, line.to_string()));
        }
    }
    out
}

/// The lines of `content` that carry text once HTML comments (which may span lines) are removed,
/// with that text and trailing whitespace stripped.
pub fn meaningful_lines(content: &[(u32, String)]) -> Vec<(u32, String)> {
    let mut in_comment = false;
    let mut out = Vec::new();
    for (no, line) in content {
        let mut text = String::new();
        let mut rest = line.as_str();
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(end) => {
                        rest = &rest[end + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.find("<!--") {
                    Some(start) => {
                        text.push_str(&rest[..start]);
                        rest = &rest[start + 4..];
                        in_comment = true;
                    }
                    None => {
                        text.push_str(rest);
                        break;
                    }
                }
            }
        }
        let text = text.trim_end();
        if !text.trim().is_empty() {
            out.push((*no, text.to_string()));
        }
    }
    out
}

/// Whether a section's content contains only whitespace and HTML comments.
pub fn is_empty(content: &[(u32, String)]) -> bool {
    meaningful_lines(content).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(lines: &[(u32, &str)]) -> Vec<(u32, String)> {
        lines.iter().map(|(n, l)| (*n, l.to_string())).collect()
    }

    #[test]
    fn numbered_lines_handle_crlf_bom_and_trailing_newline() {
        let lines = numbered_lines("\u{feff}a\r\nb\n");
        assert_eq!(lines, vec![(1, "a"), (2, "b")]);
        assert_eq!(numbered_lines("a"), vec![(1, "a")]);
    }

    #[test]
    fn front_matter_split() {
        let lines = numbered_lines("---\na: 1\n---\nbody\n");
        let split = split_front_matter(&lines).unwrap();
        assert_eq!(split.front, vec![(2, "a: 1")]);
        assert_eq!(split.body, vec![(4, "body")]);
        assert!(split_front_matter(&numbered_lines("a: 1\n")).is_err());
        assert!(split_front_matter(&numbered_lines("---\na: 1\n")).is_err());
    }

    #[test] // rq-e9d4388a
    fn headings_in_fences_are_not_sections() {
        let lines = numbered_lines("## A\n```\n## B\n```\n~~~~md\n## C\n~~~\n~~~~\n### D\n## E\n");
        let s = sections(&lines);
        let names: Vec<_> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["A", "E"]);
        assert_eq!(s[0].content.len(), 8);
    }

    #[test]
    fn atx_heading_detection() {
        assert_eq!(atx_heading("## q-1 · A?"), Some((2, "q-1 · A?")));
        assert_eq!(atx_heading("###### Deep"), Some((6, "Deep")));
        assert_eq!(atx_heading("# Title"), Some((1, "Title")));
        assert_eq!(atx_heading("####### Seven"), None);
        assert_eq!(atx_heading("##NoSpace"), None);
        assert_eq!(atx_heading("    ## code"), None);
    }

    #[test]
    fn h2_detection() {
        assert_eq!(h2_name("## Question"), Some("Question"));
        assert_eq!(h2_name("##  Result  "), Some("Result"));
        assert_eq!(h2_name("### Sub"), None);
        assert_eq!(h2_name("##Bad"), None);
        assert_eq!(h2_name("    ## Indented code"), None);
    }

    #[test] // rq-e9d4388a
    fn emptiness_ignores_whitespace_and_comments() {
        assert!(is_empty(&owned(&[
            (1, "  "),
            (2, "<!-- a"),
            (3, "b -->"),
            (4, "")
        ])));
        assert!(!is_empty(&owned(&[(1, "<!-- a --> text")])));
        assert_eq!(
            meaningful_lines(&owned(&[(1, "x <!-- c -->  "), (2, "<!--"), (3, "-->y")])),
            vec![(1, "x".to_string()), (3, "y".to_string())]
        );
    }
}
