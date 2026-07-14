//! BigQuery profile: what a `bq load --source_format=CSV` job rejects or
//! silently rewrites. The point of this profile is turning "Error while
//! reading data" into an exact byte position before you upload anything.

use crate::diag::Diagnostic;
use crate::scan::Scan;

/// Hard per-cell limit for CSV load jobs.
pub const MAX_CELL_BYTES: usize = 100 * 1024 * 1024;
/// Column names may be at most 300 characters.
pub const MAX_COLUMN_NAME_CHARS: usize = 300;

/// Run all BigQuery checks, appending to `diags`.
pub fn check(input: &[u8], scan: &Scan, header: bool, diags: &mut Vec<Diagnostic>) {
    check_encoding(scan, diags);
    check_quoted_newlines(scan, diags);
    check_cell_sizes(scan, MAX_CELL_BYTES, diags);
    if header {
        check_column_names(input, scan, diags);
    }
}

/// BQ003 (invalid UTF-8) and BQ005 (NUL bytes) both fail the load job.
fn check_encoding(scan: &Scan, diags: &mut Vec<Diagnostic>) {
    if scan.nul_total > 0 {
        diags.push(Diagnostic::new(
            "BQ005",
            scan.nul_offsets[0],
            format!(
                "NUL byte at byte {}: the load job fails with \"Bad character (ASCII 0) \
                 encountered\"{}",
                scan.nul_offsets[0],
                more(scan.nul_total)
            ),
        ));
    }
    if scan.utf8_total > 0 {
        diags.push(Diagnostic::new(
            "BQ003",
            scan.utf8_offsets[0],
            format!(
                "invalid UTF-8 at byte {}: BigQuery decodes CSV as UTF-8 by default{}",
                scan.utf8_offsets[0],
                more(scan.utf8_total)
            ),
        ));
    }
}

/// BQ001: quoted line breaks require allow_quoted_newlines.
fn check_quoted_newlines(scan: &Scan, diags: &mut Vec<Diagnostic>) {
    for rec in &scan.records {
        for (i, field) in rec.fields.iter().enumerate() {
            if let Some(at) = field.newline_at {
                diags.push(
                    Diagnostic::new(
                        "BQ001",
                        at,
                        "line break inside a quoted field: the load fails with \"Missing \
                         close double quote\" unless allow_quoted_newlines=true is set",
                    )
                    .record(rec.index)
                    .field(i + 1),
                );
            }
        }
    }
}

/// BQ002: per-cell size limit. Takes the limit as a parameter so the check
/// is testable without allocating a 100 MB fixture.
pub fn check_cell_sizes(scan: &Scan, limit: usize, diags: &mut Vec<Diagnostic>) {
    for rec in &scan.records {
        for (i, field) in rec.fields.iter().enumerate() {
            let bytes = field.content_end - field.content_start;
            if bytes > limit {
                diags.push(
                    Diagnostic::new(
                        "BQ002",
                        field.start,
                        format!(
                            "cell is {bytes} bytes; BigQuery rejects CSV cells larger than \
                             {limit} bytes"
                        ),
                    )
                    .span(field.end - field.start)
                    .record(rec.index)
                    .field(i + 1),
                );
            }
        }
    }
}

/// BQ004: headers that schema auto-detection will rename.
fn check_column_names(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    let Some(head) = scan.records.first() else {
        return;
    };
    if head.is_blank() {
        return;
    }
    for (i, field) in head.fields.iter().enumerate() {
        let content = field.content(input);
        let name = String::from_utf8_lossy(&content);
        if !valid_column_name(&name) {
            diags.push(
                Diagnostic::new(
                    "BQ004",
                    field.start,
                    format!(
                        "header \"{name}\" is not a valid BigQuery column name (letters, \
                         digits and _ only, must not start with a digit, max \
                         {MAX_COLUMN_NAME_CHARS} chars); auto-detection will rename it"
                    ),
                )
                .span((field.end - field.start).max(1))
                .record(1)
                .field(i + 1),
            );
        }
    }
}

/// `[A-Za-z_][A-Za-z0-9_]*`, at most 300 characters.
pub fn valid_column_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if name.chars().count() > MAX_COLUMN_NAME_CHARS {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn more(total: usize) -> String {
    if total > 1 {
        format!(" ({total} total)")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan;

    fn run(input: &[u8]) -> Vec<Diagnostic> {
        let s = scan(input, b',');
        let mut diags = Vec::new();
        check(input, &s, true, &mut diags);
        diags
    }

    fn find<'a>(diags: &'a [Diagnostic], code: &str) -> Option<&'a Diagnostic> {
        diags.iter().find(|d| d.code == code)
    }

    #[test]
    fn quoted_newline_points_at_the_embedded_break() {
        let input = b"id,note\r\n1,\"line one\nline two\"\r\n";
        let d = run(input);
        let hit = find(&d, "BQ001").expect("quoted newline must be flagged");
        assert_eq!(
            hit.offset, 20,
            "the LF inside the quotes, not the record start"
        );
        assert_eq!((hit.record, hit.field), (Some(2), Some(2)));
    }

    #[test]
    fn unquoted_records_produce_no_newline_warning() {
        assert!(find(&run(b"id,note\r\n1,plain\r\n"), "BQ001").is_none());
    }

    #[test]
    fn invalid_utf8_and_nul_escalate_to_errors() {
        let d = run(b"id\r\n\xFF\x00\r\n");
        assert_eq!(
            find(&d, "BQ003").unwrap().severity,
            crate::diag::Severity::Error
        );
        assert_eq!(
            find(&d, "BQ005").unwrap().severity,
            crate::diag::Severity::Error
        );
        assert_eq!(find(&d, "BQ003").unwrap().offset, 4);
        assert_eq!(find(&d, "BQ005").unwrap().offset, 5);
    }

    #[test]
    fn cell_size_limit_is_enforced_via_injected_limit() {
        let input = b"id,blob\r\n1,\"0123456789abcdef\"\r\n";
        let s = scan(input, b',');
        let mut diags = Vec::new();
        check_cell_sizes(&s, 15, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "BQ002");
        assert!(diags[0].message.contains("16 bytes"));
        // At the limit exactly: fine.
        let mut none = Vec::new();
        check_cell_sizes(&s, 16, &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn column_name_validation_matches_bigquery_rules() {
        assert!(valid_column_name("order_id"));
        assert!(valid_column_name("_private"));
        assert!(valid_column_name("A1"));
        assert!(!valid_column_name("1st"));
        assert!(!valid_column_name("order id"));
        assert!(!valid_column_name("prix-€"));
        assert!(!valid_column_name(""));
        assert!(valid_column_name(&"a".repeat(300)));
        assert!(!valid_column_name(&"a".repeat(301)));
    }

    #[test]
    fn bad_headers_are_flagged_with_exact_offsets() {
        let input = b"order id,total,1st\r\n1,2,3\r\n";
        let d = run(input);
        let hits: Vec<_> = d.iter().filter(|d| d.code == "BQ004").collect();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].offset, 0);
        assert_eq!(hits[1].offset, 15);
        assert_eq!(hits[1].field, Some(3));
    }

    #[test]
    fn headers_are_only_checked_in_header_mode() {
        let input = b"order id,total\r\n";
        let s = scan(input, b',');
        let mut diags = Vec::new();
        check(input, &s, false, &mut diags);
        assert!(find(&diags, "BQ004").is_none());
    }
}
