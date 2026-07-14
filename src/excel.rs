//! Excel profile: what Microsoft Excel truncates, mangles or refuses when a
//! CSV is opened or imported. All limits are the documented worksheet limits
//! for Excel 2007 and later.

use crate::diag::Diagnostic;
use crate::scan::{Field, Scan};

/// Hard per-cell character limit; longer cells are truncated on import.
pub const MAX_CELL_CHARS: usize = 32_767;
/// Worksheets end at column XFD.
pub const MAX_COLUMNS: usize = 16_384;
/// Worksheets end at row 1,048,576.
pub const MAX_ROWS: usize = 1_048_576;

/// Run all Excel checks, appending to `diags`.
pub fn check(input: &[u8], scan: &Scan, header: bool, diags: &mut Vec<Diagnostic>) {
    check_sylk_trap(input, scan, diags);
    check_dimensions(scan, diags);
    check_encoding_bom(input, scan, diags);
    for rec in &scan.records {
        let is_header = header && rec.index == 1;
        for (i, field) in rec.fields.iter().enumerate() {
            check_cell_length(input, field, rec.index, i + 1, diags);
            check_formula(input, field, rec.index, i + 1, diags);
            if !is_header {
                check_leading_zeros(input, field, rec.index, i + 1, diags);
            }
        }
    }
}

/// XLS003: a file starting with the bytes `ID` is misdetected as SYLK.
fn check_sylk_trap(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    let Some(first) = scan.records.first().and_then(|r| r.fields.first()) else {
        return;
    };
    // Excel looks at the first bytes of the *file*: a quoted or BOM-prefixed
    // "ID" does not trigger the SYLK sniffer.
    if first.start == 0 && !first.quoted && input[first.start..first.end].starts_with(b"ID") {
        diags.push(
            Diagnostic::new(
                "XLS003",
                0,
                "file starts with \"ID\": Excel misdetects it as a SYLK file and refuses to \
                 open it; rename the column (e.g. \"id\") or quote it",
            )
            .span(2)
            .record(1)
            .field(1),
        );
    }
}

/// XLS004/XLS005: worksheet dimension limits.
fn check_dimensions(scan: &Scan, diags: &mut Vec<Diagnostic>) {
    if let Some(rec) = scan.records.iter().find(|r| r.fields.len() > MAX_COLUMNS) {
        let surplus = &rec.fields[MAX_COLUMNS];
        diags.push(
            Diagnostic::new(
                "XLS004",
                surplus.start,
                format!(
                    "record {} has {} fields; Excel stops at column XFD ({MAX_COLUMNS}) and \
                     drops the rest",
                    rec.index,
                    rec.fields.len()
                ),
            )
            .record(rec.index)
            .field(MAX_COLUMNS + 1),
        );
    }
    if scan.records.len() > MAX_ROWS {
        let first_lost = &scan.records[MAX_ROWS];
        diags.push(
            Diagnostic::new(
                "XLS005",
                first_lost.start,
                format!(
                    "{} records exceed Excel's {MAX_ROWS}-row worksheet; rows from here on \
                     are silently not loaded",
                    scan.records.len()
                ),
            )
            .record(first_lost.index),
        );
    }
}

/// XLS006: non-ASCII UTF-8 without a BOM is decoded with the ANSI code page.
fn check_encoding_bom(input: &[u8], scan: &Scan, diags: &mut Vec<Diagnostic>) {
    if scan.bom || scan.utf8_total > 0 {
        return; // has a BOM, or not UTF-8 at all (RFC301 covers that)
    }
    if let Some(pos) = input.iter().position(|&b| b >= 0x80) {
        diags.push(Diagnostic::new(
            "XLS006",
            pos,
            "non-ASCII UTF-8 text without a BOM: double-clicking the file makes Excel decode \
             it with the system ANSI code page (mojibake); add a BOM or import via \
             Data → From Text/CSV",
        ));
    }
}

/// XLS001: per-cell character limit.
fn check_cell_length(
    input: &[u8],
    field: &Field,
    rec: usize,
    fld: usize,
    diags: &mut Vec<Diagnostic>,
) {
    let bytes = field.content_end - field.content_start;
    if bytes <= MAX_CELL_CHARS {
        return; // chars <= bytes, so cheap short-circuit for normal cells
    }
    let content = field.content(input);
    let chars = match std::str::from_utf8(&content) {
        Ok(s) => s.chars().count(),
        Err(_) => content.len(),
    };
    if chars > MAX_CELL_CHARS {
        diags.push(
            Diagnostic::new(
                "XLS001",
                field.start,
                format!(
                    "cell is {chars} characters; Excel truncates cells at {MAX_CELL_CHARS} \
                     characters on import"
                ),
            )
            .span(field.end - field.start)
            .record(rec)
            .field(fld),
        );
    }
}

/// XLS002: formula interpretation / CSV injection.
fn check_formula(input: &[u8], field: &Field, rec: usize, fld: usize, diags: &mut Vec<Diagnostic>) {
    let content = field.content(input);
    let Some(&first) = content.first() else {
        return;
    };
    let trigger = match first {
        b'=' | b'@' => true,
        // +42 or -3.5 render as numbers, not formulas; only flag +/- when the
        // whole cell does not parse as a plain number.
        b'+' | b'-' => std::str::from_utf8(&content)
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .is_none(),
        _ => false,
    };
    if trigger {
        diags.push(
            Diagnostic::new(
                "XLS002",
                field.content_start,
                format!(
                    "cell starts with '{}': Excel interprets it as a formula (CSV injection \
                     risk); prefix with a single quote or import the column as text",
                    first as char
                ),
            )
            .record(rec)
            .field(fld),
        );
    }
}

/// XLS007: digits-only values with leading zeros lose them.
fn check_leading_zeros(
    input: &[u8],
    field: &Field,
    rec: usize,
    fld: usize,
    diags: &mut Vec<Diagnostic>,
) {
    let content = field.content(input);
    if content.len() >= 2 && content[0] == b'0' && content.iter().all(|b| b.is_ascii_digit()) {
        diags.push(
            Diagnostic::new(
                "XLS007",
                field.content_start,
                format!(
                    "\"{}\" becomes the number {}: Excel strips leading zeros even from \
                     quoted cells; import the column as text to keep them",
                    String::from_utf8_lossy(&content),
                    String::from_utf8_lossy(&content).trim_start_matches('0')
                ),
            )
            .span(content.len())
            .record(rec)
            .field(fld),
        );
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

    fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
        diags.iter().map(|d| d.code).collect()
    }

    #[test]
    fn sylk_trap_fires_only_on_a_bare_leading_id() {
        let d = run(b"ID,name\r\n1,a\r\n");
        assert!(codes(&d).contains(&"XLS003"));
        assert_eq!(d.iter().find(|d| d.code == "XLS003").unwrap().offset, 0);
        // Quoted, lowercased, or non-leading "ID" is safe.
        assert!(!codes(&run(b"\"ID\",name\r\n")).contains(&"XLS003"));
        assert!(!codes(&run(b"id,name\r\n")).contains(&"XLS003"));
        assert!(!codes(&run(b"name,ID\r\n")).contains(&"XLS003"));
        // A BOM before "ID" also defuses the sniffer.
        assert!(!codes(&run(b"\xEF\xBB\xBFID,name\r\n")).contains(&"XLS003"));
    }

    #[test]
    fn oversized_cell_is_flagged_with_char_count() {
        let mut input = b"h\r\n".to_vec();
        input.extend(std::iter::repeat(b'x').take(MAX_CELL_CHARS + 1));
        input.extend(b"\r\n");
        let d = run(&input);
        let hit = d
            .iter()
            .find(|d| d.code == "XLS001")
            .expect("must flag 32768-char cell");
        assert_eq!(hit.offset, 3);
        assert!(hit.message.contains("32768 characters"));
    }

    #[test]
    fn cell_exactly_at_the_limit_is_fine() {
        let mut input = b"h\r\n".to_vec();
        input.extend(std::iter::repeat(b'x').take(MAX_CELL_CHARS));
        input.extend(b"\r\n");
        assert!(!codes(&run(&input)).contains(&"XLS001"));
    }

    #[test]
    fn multibyte_cells_are_measured_in_characters_not_bytes() {
        // 20,000 three-byte chars = 60,000 bytes but only 20,000 characters.
        let mut input = b"h\r\n".to_vec();
        input.extend("あ".repeat(20_000).bytes());
        input.extend(b"\r\n");
        assert!(!codes(&run(&input)).contains(&"XLS001"));
    }

    #[test]
    fn formula_prefixes_are_flagged_including_inside_quotes() {
        let d = run(b"h1,h2\r\n=SUM(A1:A9),\"@cmd\"\r\n");
        let hits: Vec<_> = d.iter().filter(|d| d.code == "XLS002").collect();
        assert_eq!(
            hits.len(),
            2,
            "quoting does not stop Excel evaluating formulas"
        );
        assert_eq!(hits[0].offset, 7);
        assert_eq!(hits[1].offset, 20, "offset of @ inside the quotes");
    }

    #[test]
    fn plain_negative_numbers_are_not_formulas() {
        let d = run(b"h1,h2,h3\r\n-3.5,+42,-1e9\r\n");
        assert!(!codes(&d).contains(&"XLS002"));
        // But "-A1+B1" style is.
        assert!(codes(&run(b"h\r\n-A1+B1\r\n")).contains(&"XLS002"));
    }

    #[test]
    fn leading_zeros_flag_data_rows_but_not_headers_or_mixed_values() {
        let d = run(b"0123,zip\r\n00501,0x1f\r\n");
        let hits: Vec<_> = d.iter().filter(|d| d.code == "XLS007").collect();
        assert_eq!(hits.len(), 1, "header 0123 and non-numeric 0x1f are exempt");
        assert_eq!(hits[0].record, Some(2));
        assert!(hits[0].message.contains("00501"));
    }

    #[test]
    fn column_limit_points_at_the_first_dropped_field() {
        let row: Vec<u8> = vec![b','; MAX_COLUMNS]; // MAX_COLUMNS+1 empty fields
        let d = run(&row);
        let hit = d.iter().find(|d| d.code == "XLS004").unwrap();
        assert_eq!(hit.field, Some(MAX_COLUMNS + 1));
        assert_eq!(hit.offset, MAX_COLUMNS);
    }

    #[test]
    fn row_limit_points_at_the_first_lost_record() {
        let mut input = Vec::new();
        for i in 0..=MAX_ROWS {
            input.extend(format!("{i}\n").bytes());
        }
        let d = run(&input);
        let hit = d.iter().find(|d| d.code == "XLS005").unwrap();
        assert_eq!(hit.record, Some(MAX_ROWS + 1));
    }

    #[test]
    fn bom_advice_fires_only_for_bomless_non_ascii() {
        assert!(codes(&run("h\r\nnaïve\r\n".as_bytes())).contains(&"XLS006"));
        assert!(!codes(&run(b"h\r\nplain\r\n")).contains(&"XLS006"));
        let mut with_bom = b"\xEF\xBB\xBF".to_vec();
        with_bom.extend("h\r\nnaïve\r\n".bytes());
        assert!(!codes(&run(&with_bom)).contains(&"XLS006"));
    }
}
