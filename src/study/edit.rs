//! Byte-preserving edits to `STUDY.md`.

/// A document held as lines that keep their original line endings, so untouched lines are written
/// back byte for byte.
pub(super) struct EditableDoc {
    lines: Vec<String>,
    /// Line ending used for inserted lines: the ending of the first line.
    eol: &'static str,
    /// Index of the closing front-matter `---` line.
    close: usize,
}

impl EditableDoc {
    /// Splits a document with `---`-delimited front matter.
    pub fn new(text: &str) -> Result<Self, String> {
        let lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
        let bare = |l: &str| {
            l.trim_start_matches('\u{feff}')
                .trim_end_matches(['\n', '\r'])
                .trim_end()
                .to_string()
        };
        if lines.first().map(|l| bare(l)) != Some("---".to_string()) {
            return Err("the file does not start with a `---` front-matter delimiter".into());
        }
        let close = (1..lines.len())
            .find(|&i| bare(&lines[i]) == "---")
            .ok_or("the front matter has no closing `---` delimiter")?;
        let eol = if lines[0].ends_with("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        Ok(EditableDoc { lines, eol, close })
    }

    /// The index range `[start, end)` of a top-level front-matter key and its continuation lines
    /// (indented lines, block-list items, and blank lines).
    fn key_range(&self, key: &str) -> Option<(usize, usize)> {
        let prefix = format!("{key}:");
        let start = (1..self.close).find(|&i| self.lines[i].starts_with(&prefix))?;
        let end = (start + 1..self.close)
            .find(|&i| {
                let l = &self.lines[i];
                !(l.trim().is_empty() || l.starts_with([' ', '\t', '-']))
            })
            .unwrap_or(self.close);
        Some((start, end))
    }

    fn with_eol(&self, lines: &[String]) -> Vec<String> {
        lines.iter().map(|l| format!("{l}{}", self.eol)).collect()
    }

    /// Replaces a top-level front-matter field (with its continuation lines) by `new_lines`, or
    /// inserts them before the closing `---` if the field is absent.
    pub fn set_field(&mut self, key: &str, new_lines: &[String]) {
        let replacement = self.with_eol(new_lines);
        let (start, end) = self.key_range(key).unwrap_or((self.close, self.close));
        let removed = end - start;
        self.lines.splice(start..end, replacement.iter().cloned());
        self.close = self.close + replacement.len() - removed;
    }

    /// Replaces the content of the section whose heading is at 1-based line `heading_line`, up to
    /// (not including) the 1-based line `next_heading_line` or the end of the file.
    pub fn replace_section_content(
        &mut self,
        heading_line: u32,
        next_heading_line: Option<u32>,
        content: &[String],
    ) {
        let heading = heading_line as usize - 1;
        let end = next_heading_line.map_or(self.lines.len(), |l| l as usize - 1);
        if !self.lines[heading].ends_with('\n') {
            self.lines[heading].push_str(self.eol);
        }
        let replacement = self.with_eol(content);
        self.lines.splice(heading + 1..end, replacement);
    }

    pub fn text(&self) -> String {
        self.lines.concat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn replaces_a_field_with_continuation_lines() {
        let mut doc =
            EditableDoc::new("---\nstatus: draft\nvariants:\n- base\n  # c\nother: 1\n---\nbody")
                .unwrap();
        doc.set_field("variants", &s(&["variants: [x]"]));
        doc.set_field("status", &s(&["status: preregistered"]));
        assert_eq!(
            doc.text(),
            "---\nstatus: preregistered\nvariants: [x]\nother: 1\n---\nbody"
        );
    }

    #[test]
    fn inserts_a_missing_field_before_the_delimiter_keeping_crlf() {
        let mut doc = EditableDoc::new("---\r\na: 1\r\n---\r\nbody\r\n").unwrap();
        doc.set_field("b", &s(&["b:", "  c: 2"]));
        assert_eq!(doc.text(), "---\r\na: 1\r\nb:\r\n  c: 2\r\n---\r\nbody\r\n");
    }

    #[test]
    fn replaces_section_content() {
        let mut doc = EditableDoc::new("---\n---\n## R\nold\n\n## N\nkeep\n").unwrap();
        doc.replace_section_content(3, Some(6), &s(&["", "new", ""]));
        assert_eq!(doc.text(), "---\n---\n## R\n\nnew\n\n## N\nkeep\n");
        let mut doc = EditableDoc::new("---\n---\n## R").unwrap();
        doc.replace_section_content(3, None, &s(&["", "x"]));
        assert_eq!(doc.text(), "---\n---\n## R\n\nx\n");
    }
}
