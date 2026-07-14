//! Postgres profile: what `COPY ... FROM ... WITH (FORMAT csv)` rejects or
//! quietly reinterprets, assuming the common UTF8 server encoding.

use crate::diag::Diagnostic;
use crate::scan::Scan;

/// NAMEDATALEN - 1: identifiers are truncated to this many bytes.
pub const MAX_IDENTIFIER_BYTES: usize = 63;

/// Run all Postgres checks, appending to `diags`.
pub fn check(input: &[u8], scan: &Scan, header: bool, diags: &mut Vec<Diagnostic>) {
    check_encoding(scan, diags);
    check_end_of_data_marker(input, scan, diags);
    check_null_ambiguity(input, scan, header, diags);
    if header {
        check_identifier_lengths(input, scan, diags);
    }
    if scan.bom {
        diags.push(
            Diagnostic::new(
                "PG006",
                0,
                "UTF-8 BOM: COPY does not strip it, so it becomes the first bytes of the \
                 first field and column-name matching fails",
            )
            .span(3),
        );
    }
}

/// PG001 (NUL) and PG003 (invalid UTF-8) both abort COPY.
fn check_encoding(scan: &Scan, diags: &mut Vec<Diagnostic>) {
    if scan.nul_total > 0 {
        diags.push(Diagnostic::new(
            "PG001",
            scan.nul_offsets[0],
            format!(
                "NUL byte at byte {}: COPY fails with \"invalid byte sequence for encoding \
                 UTF8: 0x00\"{}",
                scan.nul_offsets[0],
                more(scan.nul_total)
            ),
        ));
    }
    if scan.utf8_total > 0 {
        diags.push(Diagnostic::new(
            "PG003",
            scan.utf8_offsets[0],
            format!(
                "invalid UTF-8 at byte {}: COPY aborts here under a UTF8 server encoding \
                 (declare the real encoding with COPY ... ENCODING '...' if it is not UTF-8){}",
                scan.utf8_offsets[0],
                more(scan.utf8_total)
            ),
        ));
    }
}

/// PG002: a line consisting only of `\.` is COPY's end-of-data marker.
fn check_end_of_data_marker(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    for rec in &scan.records {
        if rec.fields.len() == 1 && !rec.fields[0].quoted && rec.fields[0].raw(input) == b"\\." {
            let after = scan.records.len().saturating_sub(rec.index);
            diags.push(
                Diagnostic::new(
                    "PG002",
                    rec.start,
                    format!(
                        "line contains only \"\\.\", COPY's end-of-data marker: the {after} \
                         record(s) after this line are dropped or the load errors; quote the \
                         value as \"\\.\" to keep it"
                    ),
                )
                .span(2)
                .record(rec.index)
                .field(1),
            );
        }
    }
}

/// PG005: mixing unquoted-empty (NULL) and quoted-empty ("" = empty string)
/// fields loads identical-looking cells as different values. Reported once.
fn check_null_ambiguity(input: &[u8], scan: &Scan, header: bool, diags: &mut Vec<Diagnostic>) {
    let skip = usize::from(header);
    let mut quoted_empty: Option<(usize, usize, usize)> = None; // offset, rec, fld
    let mut unquoted_empty: Option<usize> = None;
    for rec in scan.records.iter().skip(skip) {
        if rec.is_blank() {
            continue; // a blank line is not an empty value
        }
        for (i, field) in rec.fields.iter().enumerate() {
            if field.content_start != field.content_end || !field.content(input).is_empty() {
                continue;
            }
            if field.quoted {
                if quoted_empty.is_none() {
                    quoted_empty = Some((field.start, rec.index, i + 1));
                }
            } else if unquoted_empty.is_none() {
                unquoted_empty = Some(field.start);
            }
        }
    }
    if let (Some((offset, rec, fld)), Some(bare)) = (quoted_empty, unquoted_empty) {
        diags.push(
            Diagnostic::new(
                "PG005",
                offset,
                format!(
                    "quoted empty field (loads as empty string) while the file also has \
                     unquoted empty fields (load as NULL), e.g. at byte {bare}; pick one \
                     convention or use FORCE_NULL / FORCE_NOT_NULL"
                ),
            )
            .span(2)
            .record(rec)
            .field(fld),
        );
    }
}

/// PG004: headers longer than 63 bytes are truncated as identifiers.
fn check_identifier_lengths(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    let Some(head) = scan.records.first() else {
        return;
    };
    if head.is_blank() {
        return;
    }
    for (i, field) in head.fields.iter().enumerate() {
        let content = field.content(input);
        if content.len() > MAX_IDENTIFIER_BYTES {
            diags.push(
                Diagnostic::new(
                    "PG004",
                    field.start,
                    format!(
                        "header is {} bytes; Postgres truncates identifiers to \
                         {MAX_IDENTIFIER_BYTES} bytes (NAMEDATALEN - 1)",
                        content.len()
                    ),
                )
                .span(field.end - field.start)
                .record(1)
                .field(i + 1),
            );
        }
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
    fn end_of_data_marker_reports_how_many_records_are_lost() {
        let input = b"id,c\r\n1,a\r\n\\.\r\n2,b\r\n3,c\r\n";
        let d = run(input);
        let hit = find(&d, "PG002").expect("\\. line must be flagged");
        assert_eq!(hit.offset, 11);
        assert_eq!(hit.record, Some(3));
        assert!(hit.message.contains("the 2 record(s) after"));
    }

    #[test]
    fn quoted_or_embedded_backslash_dot_is_safe() {
        // Quoted: literal data. Embedded in a wider record: not a marker line.
        assert!(find(&run(b"id\r\n\"\\.\"\r\n"), "PG002").is_none());
        assert!(find(&run(b"id,c\r\n\\.,x\r\n"), "PG002").is_none());
    }

    #[test]
    fn nul_and_invalid_utf8_are_copy_errors() {
        let d = run(b"id\r\na\x00\r\n\xC3\x28\r\n");
        assert_eq!(find(&d, "PG001").unwrap().offset, 5);
        let utf8 = find(&d, "PG003").unwrap();
        assert_eq!(utf8.offset, 8, "the 0xC3 that lacks its continuation byte");
        assert_eq!(utf8.severity, crate::diag::Severity::Error);
    }

    #[test]
    fn identifier_truncation_is_measured_in_bytes_not_chars() {
        // 22 three-byte chars = 66 bytes > 63, though only 22 characters.
        let long = "あ".repeat(22);
        let input = format!("{long},b\r\n1,2\r\n");
        let hit = run(input.as_bytes());
        let hit = find(&hit, "PG004").unwrap();
        assert!(hit.message.contains("66 bytes"));
        // 63 ASCII bytes exactly: fine.
        let ok = format!("{},b\r\n1,2\r\n", "a".repeat(63));
        assert!(find(&run(ok.as_bytes()), "PG004").is_none());
    }

    #[test]
    fn null_ambiguity_fires_only_when_both_styles_appear() {
        let mixed = b"a,b,c\r\n1,\"\",3\r\n4,,6\r\n";
        let d = run(mixed);
        let hit = find(&d, "PG005").expect("mixed empty styles must be flagged");
        assert_eq!(hit.offset, 9, "points at the quoted empty field");
        assert!(hit.message.contains("byte 17"), "{}", hit.message);
        assert_eq!(
            d.iter().filter(|x| x.code == "PG005").count(),
            1,
            "reported once"
        );

        assert!(find(&run(b"a,b\r\n1,\"\"\r\n2,\"\"\r\n"), "PG005").is_none());
        assert!(find(&run(b"a,b\r\n1,\r\n2,\r\n"), "PG005").is_none());
    }

    #[test]
    fn header_row_is_exempt_from_null_ambiguity() {
        // The empty header is RFC202's business, not PG005's.
        assert!(find(&run(b"a,\"\"\r\n1,x\r\n2,\r\n"), "PG005").is_none());
    }

    #[test]
    fn bom_warning_fires_only_with_a_bom() {
        assert!(find(&run(b"\xEF\xBB\xBFid\r\n1\r\n"), "PG006").is_some());
        assert!(find(&run(b"id\r\n1\r\n"), "PG006").is_none());
    }
}
