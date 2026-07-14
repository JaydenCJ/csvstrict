//! Diagnostic model: severities, byte-anchored diagnostics, and the line
//! index that converts byte offsets into 1-based line/column positions.

use crate::codes;

/// Diagnostic severity, ordered so `Error > Warning > Info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    /// Lowercase label used in both the human and JSON reporters.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// A single finding, anchored to an exact byte offset in the input.
///
/// `severity` and `profile` are not chosen by call sites: they come from the
/// central registry in [`codes`], so a code can never be reported with an
/// inconsistent severity in two places.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Stable diagnostic code, e.g. `RFC001` or `PG002`.
    pub code: &'static str,
    pub severity: Severity,
    /// Profile that owns the code (`rfc4180`, `excel`, `bigquery`, `postgres`).
    pub profile: &'static str,
    /// 0-based byte offset of the first offending byte.
    pub offset: usize,
    /// Length in bytes of the offending span (>= 1; drives the caret width).
    pub len: usize,
    /// 1-based record number (the header, if present, is record 1).
    pub record: Option<usize>,
    /// 1-based field number within the record.
    pub field: Option<usize>,
    /// Human-readable, situation-specific message.
    pub message: String,
}

impl Diagnostic {
    /// Create a diagnostic for a registered code. Panics on unknown codes —
    /// every emitted code must exist in [`codes::CODES`] so that `explain`
    /// and the docs stay complete.
    pub fn new(code: &str, offset: usize, message: impl Into<String>) -> Self {
        let info = codes::lookup(code)
            .unwrap_or_else(|| panic!("diagnostic code {code} missing from codes::CODES"));
        Diagnostic {
            code: info.code,
            severity: info.severity,
            profile: info.profile,
            offset,
            len: 1,
            record: None,
            field: None,
            message: message.into(),
        }
    }

    /// Set the byte length of the offending span (clamped to >= 1).
    pub fn span(mut self, len: usize) -> Self {
        self.len = len.max(1);
        self
    }

    /// Attach the 1-based record number.
    pub fn record(mut self, record: usize) -> Self {
        self.record = Some(record);
        self
    }

    /// Attach the 1-based field number.
    pub fn field(mut self, field: usize) -> Self {
        self.field = Some(field);
        self
    }
}

/// Precomputed line-start table for O(log n) offset → line/column lookups.
///
/// Lines are terminated by `\n`, `\r\n`, or a lone `\r`; columns are 1-based
/// *byte* columns within the line, which keeps positions exact even when the
/// text is not valid UTF-8.
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(input: &[u8]) -> Self {
        let mut starts = vec![0];
        let mut i = 0;
        while i < input.len() {
            match input[i] {
                b'\n' => {
                    starts.push(i + 1);
                    i += 1;
                }
                b'\r' => {
                    let step = if input.get(i + 1) == Some(&b'\n') {
                        2
                    } else {
                        1
                    };
                    starts.push(i + step);
                    i += step;
                }
                _ => i += 1,
            }
        }
        LineIndex {
            starts,
            len: input.len(),
        }
    }

    /// Map a byte offset to a `(line, byte_column)` pair, both 1-based.
    /// Offsets at or past EOF resolve to one past the end of the last line.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.len);
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        (line, offset - self.starts[line - 1] + 1)
    }

    /// Byte range `(start, end)` of a 1-based line, excluding its terminator.
    pub fn line_span(&self, line: usize, input: &[u8]) -> (usize, usize) {
        let start = self.starts[line - 1];
        let mut end = if line < self.starts.len() {
            self.starts[line]
        } else {
            self.len
        };
        while end > start && (input[end - 1] == b'\n' || input[end - 1] == b'\r') {
            end -= 1;
        }
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_error_above_warning_above_info() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn diagnostic_inherits_severity_and_profile_from_registry() {
        let d = Diagnostic::new("RFC001", 7, "unterminated");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.profile, "rfc4180");
        assert_eq!(d.offset, 7);
        assert_eq!(d.len, 1);
    }

    #[test]
    fn diagnostic_builders_set_span_record_field() {
        let d = Diagnostic::new("RFC201", 3, "count")
            .span(4)
            .record(2)
            .field(3);
        assert_eq!((d.len, d.record, d.field), (4, Some(2), Some(3)));
    }

    #[test]
    fn line_index_maps_offsets_across_lf_and_crlf() {
        let idx = LineIndex::new(b"ab\ncd\r\nef");
        assert_eq!(idx.position(0), (1, 1));
        assert_eq!(idx.position(2), (1, 3)); // the \n itself is column 3 of line 1
        assert_eq!(idx.position(3), (2, 1));
        assert_eq!(idx.position(7), (3, 1));
        assert_eq!(idx.position(8), (3, 2));
    }

    #[test]
    fn line_index_handles_lone_cr_terminators() {
        let idx = LineIndex::new(b"a\rb\rc");
        assert_eq!(idx.position(2), (2, 1));
        assert_eq!(idx.position(4), (3, 1));
    }

    #[test]
    fn line_index_clamps_offsets_past_eof() {
        let idx = LineIndex::new(b"ab\ncd");
        assert_eq!(idx.position(99), (2, 3));
    }

    #[test]
    fn line_span_excludes_terminators() {
        let input = b"ab\r\ncde\nf";
        let idx = LineIndex::new(input);
        assert_eq!(idx.line_span(1, input), (0, 2));
        assert_eq!(idx.line_span(2, input), (4, 7));
        assert_eq!(idx.line_span(3, input), (8, 9));
    }
}
