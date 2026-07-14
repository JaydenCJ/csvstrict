//! Byte-level RFC 4180 scanner.
//!
//! Tokenizes raw bytes into records and fields, keeping the exact byte span
//! of everything, and emits structural diagnostics (quoting and termination
//! problems) with recovery: one missing quote produces one diagnostic at the
//! offending byte instead of cascading to the end of the file.

use crate::diag::Diagnostic;

/// How many concrete offsets to keep for repeated byte-level findings
/// (NULs, invalid UTF-8 sequences); totals are always exact.
pub const OFFSET_CAP: usize = 8;

/// One field, with byte spans into the original input.
#[derive(Debug, Clone)]
pub struct Field {
    /// Offset of the first byte of the field (the opening quote, if quoted).
    pub start: usize,
    /// One past the last byte of the field (the closing quote, if quoted).
    pub end: usize,
    /// Span of the field *content*, excluding the enclosing quotes.
    pub content_start: usize,
    pub content_end: usize,
    pub quoted: bool,
    /// Content contains at least one escaped quote (`""`).
    pub escaped_quotes: bool,
    /// Offset of the first CR or LF inside a quoted field, if any.
    pub newline_at: Option<usize>,
}

impl Field {
    /// Raw content bytes (quotes stripped, `""` still doubled).
    pub fn raw<'a>(&self, input: &'a [u8]) -> &'a [u8] {
        &input[self.content_start..self.content_end]
    }

    /// Decoded content bytes: quotes stripped and `""` collapsed to `"`.
    pub fn content(&self, input: &[u8]) -> Vec<u8> {
        let raw = self.raw(input);
        if !self.escaped_quotes {
            return raw.to_vec();
        }
        let mut out = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'"' && raw.get(i + 1) == Some(&b'"') {
                out.push(b'"');
                i += 2;
            } else {
                out.push(raw[i]);
                i += 1;
            }
        }
        out
    }
}

/// One record: a run of fields separated by the delimiter.
#[derive(Debug, Clone)]
pub struct Record {
    /// 1-based record number (the header, if any, is record 1).
    pub index: usize,
    /// Byte span of the record, excluding its line terminator.
    pub start: usize,
    pub end: usize,
    pub fields: Vec<Field>,
}

impl Record {
    /// True for a blank line: a single empty unquoted field.
    pub fn is_blank(&self) -> bool {
        self.fields.len() == 1
            && !self.fields[0].quoted
            && self.fields[0].start == self.fields[0].end
    }
}

/// Result of scanning one input buffer.
pub struct Scan {
    pub records: Vec<Record>,
    /// Structural diagnostics found during tokenization.
    pub diags: Vec<Diagnostic>,
    /// Input starts with a UTF-8 byte order mark.
    pub bom: bool,
    /// Input ends with a record terminator.
    pub ends_with_newline: bool,
    /// First offsets (capped at [`OFFSET_CAP`]) and exact total of NUL bytes.
    pub nul_offsets: Vec<usize>,
    pub nul_total: usize,
    /// First offsets (capped) and exact total of invalid UTF-8 sequences.
    pub utf8_offsets: Vec<usize>,
    pub utf8_total: usize,
    /// Count and first offset of records terminated by bare LF.
    pub lf_endings: (usize, usize),
    /// Count and first offset of records terminated by a lone CR.
    pub cr_endings: (usize, usize),
}

enum Term {
    Delim,
    /// A record terminator: kind + offset of its first byte.
    Newline(NlKind, usize),
    Eof,
}

#[derive(PartialEq)]
enum NlKind {
    Crlf,
    Lf,
    Cr,
}

/// Scan `input` into records/fields with structural diagnostics.
pub fn scan(input: &[u8], delimiter: u8) -> Scan {
    let bom = input.starts_with(&[0xEF, 0xBB, 0xBF]);
    let mut pos = if bom { 3 } else { 0 };
    let mut records = Vec::new();
    let mut diags = Vec::new();
    let mut lf = (0usize, 0usize);
    let mut cr = (0usize, 0usize);
    let mut ends_with_newline = pos >= input.len();

    while pos < input.len() {
        let rec_start = pos;
        let rec_index = records.len() + 1;
        let mut fields = Vec::new();
        let rec_end;
        loop {
            let (field, term, next) = scan_field(
                input,
                pos,
                delimiter,
                rec_index,
                fields.len() + 1,
                &mut diags,
            );
            let field_end = field.end;
            fields.push(field);
            pos = next;
            match term {
                Term::Delim => continue,
                Term::Newline(kind, at) => {
                    match kind {
                        NlKind::Crlf => {}
                        NlKind::Lf => {
                            if lf.0 == 0 {
                                lf.1 = at;
                            }
                            lf.0 += 1;
                        }
                        NlKind::Cr => {
                            if cr.0 == 0 {
                                cr.1 = at;
                            }
                            cr.0 += 1;
                        }
                    }
                    ends_with_newline = pos >= input.len();
                    rec_end = at;
                    break;
                }
                Term::Eof => {
                    ends_with_newline = false;
                    rec_end = field_end.max(rec_start);
                    break;
                }
            }
        }
        records.push(Record {
            index: rec_index,
            start: rec_start,
            end: rec_end,
            fields,
        });
    }

    let (nul_offsets, nul_total) = find_nuls(input);
    let (utf8_offsets, utf8_total) = find_utf8_errors(input);

    Scan {
        records,
        diags,
        bom,
        ends_with_newline,
        nul_offsets,
        nul_total,
        utf8_offsets,
        utf8_total,
        lf_endings: lf,
        cr_endings: cr,
    }
}

/// Scan one field starting at `start`; returns the field, what terminated
/// it, and the offset to resume scanning at.
fn scan_field(
    input: &[u8],
    start: usize,
    delimiter: u8,
    rec: usize,
    fld: usize,
    diags: &mut Vec<Diagnostic>,
) -> (Field, Term, usize) {
    if input.get(start) == Some(&b'"') {
        scan_quoted(input, start, delimiter, rec, fld, diags)
    } else {
        scan_unquoted(input, start, delimiter, rec, fld, diags)
    }
}

fn scan_unquoted(
    input: &[u8],
    start: usize,
    delimiter: u8,
    rec: usize,
    fld: usize,
    diags: &mut Vec<Diagnostic>,
) -> (Field, Term, usize) {
    let mut i = start;
    let mut quote_reported = false;
    let field = |end: usize| Field {
        start,
        end,
        content_start: start,
        content_end: end,
        quoted: false,
        escaped_quotes: false,
        newline_at: None,
    };
    loop {
        match input.get(i) {
            None => return (field(i), Term::Eof, i),
            Some(&b) if b == delimiter => return (field(i), Term::Delim, i + 1),
            Some(&b'\n') => return (field(i), Term::Newline(NlKind::Lf, i), i + 1),
            Some(&b'\r') => {
                return if input.get(i + 1) == Some(&b'\n') {
                    (field(i), Term::Newline(NlKind::Crlf, i), i + 2)
                } else {
                    (field(i), Term::Newline(NlKind::Cr, i), i + 1)
                };
            }
            Some(&b'"') => {
                // Report once per field: a broken quote usually repeats.
                if !quote_reported {
                    let hint = if i == start {
                        // Unreachable in practice (a leading quote enters
                        // scan_quoted), but kept for safety.
                        String::from("bare double quote")
                    } else if input[start..i].iter().all(|&b| b == b' ' || b == b'\t') {
                        format!(
                            "quoted field starts with whitespace at byte {start}; the opening \
                             quote must be the first byte of the field"
                        )
                    } else {
                        String::from(
                            "bare double quote in unquoted field; fields containing quotes \
                             must be fully quoted, with inner quotes doubled (\"\")",
                        )
                    };
                    diags.push(Diagnostic::new("RFC004", i, hint).record(rec).field(fld));
                    quote_reported = true;
                }
                i += 1;
            }
            Some(_) => i += 1,
        }
    }
}

fn scan_quoted(
    input: &[u8],
    start: usize,
    delimiter: u8,
    rec: usize,
    fld: usize,
    diags: &mut Vec<Diagnostic>,
) -> (Field, Term, usize) {
    let content_start = start + 1;
    let mut i = content_start;
    let mut escaped_quotes = false;
    let mut newline_at = None;
    let make = |content_end: usize, end: usize, escaped: bool, nl: Option<usize>| Field {
        start,
        end,
        content_start,
        content_end,
        quoted: true,
        escaped_quotes: escaped,
        newline_at: nl,
    };
    loop {
        match input.get(i) {
            None => {
                diags.push(
                    Diagnostic::new(
                        "RFC001",
                        start,
                        format!(
                            "unterminated quoted field: quote opened at byte {start} is never \
                             closed before end of input"
                        ),
                    )
                    .record(rec)
                    .field(fld),
                );
                return (make(i, i, escaped_quotes, newline_at), Term::Eof, i);
            }
            Some(&b'"') => {
                let close = |end: usize| make(i, end, escaped_quotes, newline_at);
                match input.get(i + 1) {
                    Some(&b'"') => {
                        escaped_quotes = true;
                        i += 2;
                    }
                    None => return (close(i + 1), Term::Eof, i + 1),
                    Some(&b) if b == delimiter => return (close(i + 1), Term::Delim, i + 2),
                    Some(&b'\n') => return (close(i + 1), Term::Newline(NlKind::Lf, i + 1), i + 2),
                    Some(&b'\r') => {
                        return if input.get(i + 2) == Some(&b'\n') {
                            (close(i + 1), Term::Newline(NlKind::Crlf, i + 1), i + 3)
                        } else {
                            (close(i + 1), Term::Newline(NlKind::Cr, i + 1), i + 2)
                        };
                    }
                    Some(&b' ') | Some(&b'\t') => {
                        // Padding after a closing quote, or an unescaped inner
                        // quote — decide by looking past the whitespace run.
                        let mut j = i + 1;
                        while matches!(input.get(j), Some(&b' ') | Some(&b'\t')) {
                            j += 1;
                        }
                        let pad = Diagnostic::new(
                            "RFC003",
                            i + 1,
                            format!(
                                "{} byte(s) of whitespace between the closing quote and the \
                                 {}; strict parsers reject this padding",
                                j - (i + 1),
                                if input.get(j).is_some_and(|&b| b == delimiter) {
                                    "delimiter"
                                } else {
                                    "record terminator"
                                }
                            ),
                        )
                        .span(j - (i + 1))
                        .record(rec)
                        .field(fld);
                        match input.get(j) {
                            None => {
                                diags.push(pad);
                                return (close(j), Term::Eof, j);
                            }
                            Some(&b) if b == delimiter => {
                                diags.push(pad);
                                return (close(j), Term::Delim, j + 1);
                            }
                            Some(&b'\n') => {
                                diags.push(pad);
                                return (close(j), Term::Newline(NlKind::Lf, j), j + 1);
                            }
                            Some(&b'\r') => {
                                diags.push(pad);
                                return if input.get(j + 1) == Some(&b'\n') {
                                    (close(j), Term::Newline(NlKind::Crlf, j), j + 2)
                                } else {
                                    (close(j), Term::Newline(NlKind::Cr, j), j + 1)
                                };
                            }
                            Some(_) => {
                                // Not padding: an unescaped quote mid-field.
                                diags.push(unescaped_quote(i, rec, fld));
                                i += 1;
                            }
                        }
                    }
                    Some(_) => {
                        diags.push(unescaped_quote(i, rec, fld));
                        i += 1; // recover: treat the quote as literal data
                    }
                }
            }
            Some(&b'\n') | Some(&b'\r') => {
                if newline_at.is_none() {
                    newline_at = Some(i);
                }
                i += 1;
            }
            Some(_) => i += 1,
        }
    }
}

fn unescaped_quote(at: usize, rec: usize, fld: usize) -> Diagnostic {
    Diagnostic::new(
        "RFC002",
        at,
        "unescaped double quote inside quoted field; write \"\" for a literal quote",
    )
    .record(rec)
    .field(fld)
}

fn find_nuls(input: &[u8]) -> (Vec<usize>, usize) {
    let mut offsets = Vec::new();
    let mut total = 0;
    for (i, &b) in input.iter().enumerate() {
        if b == 0 {
            total += 1;
            if offsets.len() < OFFSET_CAP {
                offsets.push(i);
            }
        }
    }
    (offsets, total)
}

fn find_utf8_errors(input: &[u8]) -> (Vec<usize>, usize) {
    let mut offsets = Vec::new();
    let mut total = 0;
    let mut base = 0;
    let mut rest = input;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(_) => break,
            Err(e) => {
                let bad = e.valid_up_to();
                total += 1;
                if offsets.len() < OFFSET_CAP {
                    offsets.push(base + bad);
                }
                // error_len() is None only for a truncated sequence at EOF.
                let skip = bad + e.error_len().unwrap_or(rest.len() - bad);
                base += skip;
                rest = &rest[skip..];
            }
        }
    }
    (offsets, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(input: &[u8]) -> Vec<Vec<u8>> {
        let s = scan(input, b',');
        assert_eq!(s.records.len(), 1, "expected a single record");
        s.records[0]
            .fields
            .iter()
            .map(|f| f.content(input))
            .collect()
    }

    fn codes(input: &[u8]) -> Vec<&'static str> {
        scan(input, b',').diags.iter().map(|d| d.code).collect()
    }

    #[test]
    fn plain_fields_split_on_delimiter() {
        assert_eq!(
            fields(b"a,bc,def"),
            vec![b"a".to_vec(), b"bc".to_vec(), b"def".to_vec()]
        );
    }

    #[test]
    fn empty_fields_are_preserved() {
        assert_eq!(
            fields(b",,"),
            vec![b"".to_vec(), b"".to_vec(), b"".to_vec()]
        );
    }

    #[test]
    fn quoted_field_may_contain_delimiter_and_newline() {
        let input = b"\"a,b\",\"c\nd\"";
        let f = fields(input);
        assert_eq!(f, vec![b"a,b".to_vec(), b"c\nd".to_vec()]);
        let s = scan(input, b',');
        assert_eq!(s.records[0].fields[1].newline_at, Some(8));
        assert!(s.diags.is_empty());
    }

    #[test]
    fn escaped_quotes_are_collapsed_in_content() {
        assert_eq!(fields(b"\"5\"\" nails\""), vec![b"5\" nails".to_vec()]);
        assert!(codes(b"\"5\"\" nails\"").is_empty());
    }

    #[test]
    fn field_spans_are_byte_exact() {
        let input = b"ab,\"cd\",e";
        let s = scan(input, b',');
        let f = &s.records[0].fields;
        assert_eq!((f[0].start, f[0].end), (0, 2));
        assert_eq!((f[1].start, f[1].end), (3, 7));
        assert_eq!((f[1].content_start, f[1].content_end), (4, 6));
        assert_eq!((f[2].start, f[2].end), (8, 9));
    }

    #[test]
    fn crlf_and_lf_both_terminate_records() {
        let s = scan(b"a,b\r\nc,d\ne,f", b',');
        assert_eq!(s.records.len(), 3);
        assert_eq!(s.records[1].start, 5);
        assert_eq!(s.records[2].start, 9);
        assert_eq!(s.lf_endings, (1, 8));
        assert_eq!(s.cr_endings.0, 0);
    }

    #[test]
    fn lone_cr_terminates_a_record_and_is_counted() {
        let s = scan(b"a\rb", b',');
        assert_eq!(s.records.len(), 2);
        assert_eq!(s.cr_endings, (1, 1));
    }

    #[test]
    fn trailing_newline_does_not_create_a_phantom_record() {
        let s = scan(b"a,b\n", b',');
        assert_eq!(s.records.len(), 1);
        assert!(s.ends_with_newline);
        let s2 = scan(b"a,b", b',');
        assert!(!s2.ends_with_newline);
    }

    #[test]
    fn unterminated_quote_reports_the_opening_byte() {
        let input = b"a,\"oops\nb,c";
        let s = scan(input, b',');
        let d = &s.diags[0];
        assert_eq!(d.code, "RFC001");
        assert_eq!(d.offset, 2, "must point at the opening quote, not EOF");
        assert_eq!((d.record, d.field), (Some(1), Some(2)));
        // The swallowed newline is still visible as field data.
        assert_eq!(s.records.len(), 1);
    }

    #[test]
    fn unescaped_inner_quote_recovers_within_the_field() {
        let input = b"a,\"mis\"quoted\",z";
        let s = scan(input, b',');
        assert_eq!(s.diags.len(), 1);
        assert_eq!(s.diags[0].code, "RFC002");
        assert_eq!(s.diags[0].offset, 6);
        // Recovery keeps the record intact: 3 fields, quote kept as data.
        assert_eq!(s.records[0].fields.len(), 3);
        assert_eq!(s.records[0].fields[1].content(input), b"mis\"quoted");
    }

    #[test]
    fn bare_quote_in_unquoted_field_is_reported_once_per_field() {
        let input = b"gizmo \"pro\" edition,1";
        let s = scan(input, b',');
        let quotes: Vec<_> = s.diags.iter().filter(|d| d.code == "RFC004").collect();
        assert_eq!(
            quotes.len(),
            1,
            "repeat quotes in one field collapse to one diagnostic"
        );
        assert_eq!(quotes[0].offset, 6);
        assert_eq!(s.records[0].fields.len(), 2);
    }

    #[test]
    fn whitespace_before_opening_quote_gets_a_targeted_hint() {
        let input = b"a, \"padded\"";
        let s = scan(input, b',');
        assert_eq!(s.diags[0].code, "RFC004");
        assert!(
            s.diags[0].message.contains("whitespace"),
            "{}",
            s.diags[0].message
        );
    }

    #[test]
    fn whitespace_after_closing_quote_is_padding_not_data() {
        let input = b"\"a\"  ,b";
        let s = scan(input, b',');
        assert_eq!(s.diags.len(), 1);
        let d = &s.diags[0];
        assert_eq!((d.code, d.offset, d.len), ("RFC003", 3, 2));
        assert_eq!(s.records[0].fields.len(), 2);
        assert_eq!(s.records[0].fields[0].content(input), b"a");
    }

    #[test]
    fn quote_then_whitespace_then_data_is_an_unescaped_quote() {
        let input = b"\"5\" nails\",x";
        let s = scan(input, b',');
        assert_eq!(s.diags[0].code, "RFC002");
        assert_eq!(s.diags[0].offset, 2);
        assert_eq!(s.records[0].fields[0].content(input), b"5\" nails");
    }

    #[test]
    fn bom_is_detected_and_excluded_from_the_first_field() {
        let input = b"\xEF\xBB\xBFid,name";
        let s = scan(input, b',');
        assert!(s.bom);
        assert_eq!(s.records[0].fields[0].content(input), b"id");
        assert_eq!(s.records[0].fields[0].start, 3);
    }

    #[test]
    fn custom_delimiter_is_honored_and_comma_becomes_data() {
        let s = scan(b"a;b,c;d", b';');
        let input = b"a;b,c;d";
        let f: Vec<_> = s.records[0]
            .fields
            .iter()
            .map(|f| f.content(input))
            .collect();
        assert_eq!(f, vec![b"a".to_vec(), b"b,c".to_vec(), b"d".to_vec()]);
    }

    #[test]
    fn nul_bytes_are_counted_with_capped_offsets() {
        let mut input = vec![b'a', 0, b'b'];
        input.extend(std::iter::repeat(0).take(20));
        let (offsets, total) = find_nuls(&input);
        assert_eq!(total, 21);
        assert_eq!(offsets.len(), OFFSET_CAP);
        assert_eq!(offsets[0], 1);
    }

    #[test]
    fn invalid_utf8_sequences_are_located_precisely() {
        // 0xFF at 2, then a truncated 3-byte sequence at the end.
        let input = b"ab\xFFcd\xE2\x82";
        let (offsets, total) = find_utf8_errors(input);
        assert_eq!(total, 2);
        assert_eq!(offsets, vec![2, 5]);
        assert_eq!(find_utf8_errors(b"plain ascii").1, 0);
        assert_eq!(find_utf8_errors("héllo".as_bytes()).1, 0);
    }

    #[test]
    fn blank_record_detection() {
        let s = scan(b"a,b\n\nc,d", b',');
        assert_eq!(s.records.len(), 3);
        assert!(s.records[1].is_blank());
        assert!(!s.records[0].is_blank());
    }

    #[test]
    fn empty_input_yields_no_records_and_no_diags() {
        let s = scan(b"", b',');
        assert!(s.records.is_empty());
        assert!(s.diags.is_empty());
        assert!(
            s.ends_with_newline,
            "empty input has nothing left unterminated"
        );
    }
}
