//! Base (profile-independent) checks layered on top of the scanner:
//! record shape, headers, encoding, line endings, file shape.

use crate::diag::Diagnostic;
use crate::scan::Scan;

/// Run all base checks, appending to `diags`.
pub fn check(input: &[u8], scan: &Scan, header: bool, diags: &mut Vec<Diagnostic>) {
    check_field_counts(scan, header, diags);
    if header {
        check_headers(input, scan, diags);
    }
    check_encoding(scan, diags);
    check_file_shape(input, scan, diags);
}

/// RFC201 (field count mismatch) and RFC104 (blank line).
fn check_field_counts(scan: &Scan, header: bool, diags: &mut Vec<Diagnostic>) {
    let Some(first) = scan.records.first() else {
        return;
    };
    let expected = first.fields.len();
    let baseline = if header { "the header" } else { "record 1" };
    for rec in &scan.records[1..] {
        let got = rec.fields.len();
        if got == expected {
            continue;
        }
        if rec.is_blank() {
            diags.push(
                Diagnostic::new(
                    "RFC104",
                    rec.start,
                    format!(
                        "blank line read as record {} with a single empty field",
                        rec.index
                    ),
                )
                .record(rec.index),
            );
            continue;
        }
        // Point at the first surplus field, or at the record end when short —
        // "column 4 is missing" is only findable at the end of the record.
        let (offset, len) = if got > expected {
            let f = &rec.fields[expected];
            (f.start, (f.end - f.start).max(1))
        } else {
            (rec.end, 1)
        };
        diags.push(
            Diagnostic::new(
                "RFC201",
                offset,
                format!(
                    "record {} has {} field(s), expected {} from {}",
                    rec.index, got, expected, baseline
                ),
            )
            .span(len)
            .record(rec.index)
            .field(expected.min(got) + usize::from(got > expected)),
        );
    }
}

/// RFC202 (empty header) and RFC203 (duplicate header).
fn check_headers(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    let Some(head) = scan.records.first() else {
        return;
    };
    // A file that is a single blank record has no headers worth checking.
    if head.is_blank() {
        return;
    }
    let names: Vec<Vec<u8>> = head.fields.iter().map(|f| f.content(input)).collect();
    for (i, (field, name)) in head.fields.iter().zip(&names).enumerate() {
        if name.is_empty() {
            diags.push(
                Diagnostic::new("RFC202", field.start, format!("header {} is empty", i + 1))
                    .record(1)
                    .field(i + 1),
            );
            continue;
        }
        if let Some(first) = names[..i].iter().position(|n| n == name) {
            diags.push(
                Diagnostic::new(
                    "RFC203",
                    field.start,
                    format!(
                        "duplicate header name \"{}\" (first used by header {})",
                        String::from_utf8_lossy(name),
                        first + 1
                    ),
                )
                .span((field.end - field.start).max(1))
                .record(1)
                .field(i + 1),
            );
        }
    }
}

/// RFC005 (NUL bytes) and RFC301 (invalid UTF-8), each reported once with
/// the first exact offset and the total count.
fn check_encoding(scan: &Scan, diags: &mut Vec<Diagnostic>) {
    if scan.nul_total > 0 {
        diags.push(Diagnostic::new(
            "RFC005",
            scan.nul_offsets[0],
            format!(
                "NUL byte (0x00) at byte {}{}; if the file is UTF-16, re-encode it as UTF-8",
                scan.nul_offsets[0],
                more(scan.nul_total)
            ),
        ));
    }
    if scan.utf8_total > 0 {
        diags.push(Diagnostic::new(
            "RFC301",
            scan.utf8_offsets[0],
            format!(
                "invalid UTF-8 sequence at byte {}{}",
                scan.utf8_offsets[0],
                more(scan.utf8_total)
            ),
        ));
    }
}

/// RFC101/RFC102 (line-ending conventions), RFC103 (missing final newline),
/// RFC105 (BOM).
fn check_file_shape(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    if scan.bom {
        diags.push(
            Diagnostic::new(
                "RFC105",
                0,
                "UTF-8 byte order mark (EF BB BF) at start of file",
            )
            .span(3),
        );
    }
    let (lf_count, lf_first) = scan.lf_endings;
    if lf_count > 0 {
        diags.push(Diagnostic::new(
            "RFC101",
            lf_first,
            format!(
                "{lf_count} record(s) terminated by bare LF; RFC 4180 specifies CRLF \
                 (first occurrence shown)"
            ),
        ));
    }
    let (cr_count, cr_first) = scan.cr_endings;
    if cr_count > 0 {
        diags.push(Diagnostic::new(
            "RFC102",
            cr_first,
            format!(
                "{cr_count} record(s) terminated by a lone CR; many parsers will not split \
                 these lines (first occurrence shown)"
            ),
        ));
    }
    if !scan.records.is_empty() && !scan.ends_with_newline {
        diags.push(Diagnostic::new(
            "RFC103",
            input.len().saturating_sub(1),
            "no line break after the last record (permitted by RFC 4180; some tools mind)",
        ));
    }
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

    fn run(input: &[u8], header: bool) -> Vec<Diagnostic> {
        let s = scan(input, b',');
        let mut diags = Vec::new();
        check(input, &s, header, &mut diags);
        diags
    }

    fn find<'a>(diags: &'a [Diagnostic], code: &str) -> Option<&'a Diagnostic> {
        diags.iter().find(|d| d.code == code)
    }

    #[test]
    fn consistent_records_produce_no_count_diagnostics() {
        let d = run(b"a,b,c\r\n1,2,3\r\n4,5,6\r\n", true);
        assert!(find(&d, "RFC201").is_none());
        assert!(find(&d, "RFC104").is_none());
    }

    #[test]
    fn short_record_points_at_the_record_end() {
        let input = b"a,b,c\r\n1,2\r\n";
        let d = run(input, true);
        let rfc = find(&d, "RFC201").expect("short record must be flagged");
        assert_eq!(rfc.offset, 10, "points where the missing field would start");
        assert_eq!(rfc.record, Some(2));
        assert!(rfc.message.contains("has 2 field(s), expected 3"));
    }

    #[test]
    fn long_record_points_at_the_first_surplus_field() {
        let input = b"a,b\r\n1,2,3,4\r\n";
        let d = run(input, true);
        let rfc = find(&d, "RFC201").unwrap();
        assert_eq!(rfc.offset, 9, "byte offset of the surplus field \"3\"");
        assert_eq!(rfc.field, Some(3));
    }

    #[test]
    fn no_header_mode_uses_record_1_as_the_baseline() {
        let d = run(b"1,2\r\n3,4,5\r\n", false);
        assert!(find(&d, "RFC201")
            .unwrap()
            .message
            .contains("from record 1"));
        assert!(find(&d, "RFC202").is_none(), "header checks must be off");
    }

    #[test]
    fn blank_line_is_rfc104_not_a_count_mismatch() {
        let d = run(b"a,b\r\n\r\n1,2\r\n", true);
        assert!(find(&d, "RFC104").is_some());
        assert!(find(&d, "RFC201").is_none());
    }

    #[test]
    fn empty_and_duplicate_headers_are_flagged() {
        let input = b"id,,id,name\r\n1,2,3,4\r\n";
        let d = run(input, true);
        let empty = find(&d, "RFC202").unwrap();
        assert_eq!((empty.offset, empty.field), (3, Some(2)));
        let dup = find(&d, "RFC203").unwrap();
        assert_eq!(dup.offset, 4, "flags the second \"id\", not the first");
        assert!(dup.message.contains("first used by header 1"));
    }

    #[test]
    fn quoted_headers_are_compared_by_content() {
        // "id" and id are the same name once decoded.
        let d = run(b"\"id\",id\r\n1,2\r\n", true);
        assert!(find(&d, "RFC203").is_some());
    }

    #[test]
    fn nul_and_utf8_diagnostics_carry_first_offset_and_total() {
        let input = b"a,\x00b\x00\r\nx,\xFF\r\n";
        let d = run(input, true);
        let nul = find(&d, "RFC005").unwrap();
        assert_eq!(nul.offset, 2);
        assert!(nul.message.contains("(2 total)"));
        let utf8 = find(&d, "RFC301").unwrap();
        assert_eq!(utf8.offset, 9);
    }

    #[test]
    fn line_ending_conventions_are_reported_once_with_counts() {
        let d = run(b"a,b\n1,2\n3,4\n", true);
        let lf = find(&d, "RFC101").unwrap();
        assert_eq!(lf.offset, 3, "first LF");
        assert!(lf.message.contains("3 record(s)"));
        assert_eq!(d.iter().filter(|x| x.code == "RFC101").count(), 1);
        assert!(find(&run(b"a,b\r\n1,2\r\n", true), "RFC101").is_none());
    }

    #[test]
    fn lone_cr_is_a_warning_with_first_offset() {
        let d = run(b"a,b\r1,2\r", true);
        let cr = find(&d, "RFC102").unwrap();
        assert_eq!(cr.offset, 3);
        assert_eq!(cr.severity, crate::diag::Severity::Warning);
    }

    #[test]
    fn missing_final_newline_and_bom_are_informational() {
        let d = run(b"\xEF\xBB\xBFa,b\r\n1,2", true);
        assert!(find(&d, "RFC105").is_some());
        assert!(find(&d, "RFC103").is_some());
        let clean = run(b"a,b\r\n1,2\r\n", true);
        assert!(find(&clean, "RFC103").is_none());
        assert!(find(&clean, "RFC105").is_none());
    }
}
