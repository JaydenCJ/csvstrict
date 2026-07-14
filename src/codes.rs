//! Central registry of every diagnostic code csvstrict can emit.
//!
//! One row per code: fixed severity, owning profile, a one-line title, and
//! the longer explanation shown by `csvstrict explain <CODE>`. Keeping this
//! in one table guarantees `explain`, `profiles`, the reporters and the docs
//! can never disagree about what a code means.

use crate::diag::Severity;

/// Registry entry for one diagnostic code.
pub struct CodeInfo {
    pub code: &'static str,
    pub severity: Severity,
    /// Profile that owns the check: `rfc4180`, `excel`, `bigquery`, `postgres`.
    pub profile: &'static str,
    /// One-line summary (shown by `csvstrict profiles`).
    pub title: &'static str,
    /// Longer explanation with the concrete consumer behavior and a fix.
    pub detail: &'static str,
}

use Severity::{Error, Info, Warning};

macro_rules! code {
    ($code:literal, $sev:expr, $profile:literal, $title:literal, $detail:literal) => {
        CodeInfo {
            code: $code,
            severity: $sev,
            profile: $profile,
            title: $title,
            detail: $detail,
        }
    };
}

/// Every code, grouped by profile. Order here is the order `profiles` prints.
pub const CODES: &[CodeInfo] =
    &[
        // --- RFC 4180 structure -------------------------------------------------
        code!(
            "RFC001",
            Error,
            "rfc4180",
            "unterminated quoted field",
            "A field opened with a double quote but the file ended before the closing quote. \
         Every byte after the opening quote was swallowed into this field, which is why a \
         single missing quote often surfaces as a bogus error on the last line in other tools. \
         Fix: close the quote, or escape literal quotes inside the field by doubling them (\"\")."
        ),
        code!(
            "RFC002",
            Error,
            "rfc4180",
            "unescaped double quote inside quoted field",
            "Inside a quoted field, a double quote must be escaped by doubling it (\"\"). A lone \
         quote here is either an unescaped literal quote or a closing quote followed by stray \
         data; strict parsers reject both. csvstrict recovers by treating the quote as data."
        ),
        code!(
            "RFC003",
            Warning,
            "rfc4180",
            "whitespace between closing quote and delimiter",
            "A quoted field is followed by spaces or tabs before the delimiter or line break. \
         RFC 4180 has no concept of padding: strict parsers reject it, lenient ones silently \
         attach the whitespace to the value. Remove the padding."
        ),
        code!(
            "RFC004",
            Error,
            "rfc4180",
            "bare double quote in unquoted field",
            "An unquoted field contains a double quote. Per RFC 4180, any field containing quotes \
         must be entirely enclosed in quotes, with inner quotes doubled (\"\"). Note that the \
         opening quote must be the first byte of the field: a space before it makes the whole \
         field unquoted."
        ),
        code!(
            "RFC005",
            Error,
            "rfc4180",
            "NUL byte in input",
            "The input contains 0x00 bytes. No mainstream CSV consumer accepts NUL: BigQuery and \
         Postgres COPY reject the load and many parsers truncate at the first NUL. If the file \
         is UTF-16 (a common source of interleaved NULs), re-encode it as UTF-8."
        ),
        // --- RFC 4180 file shape ------------------------------------------------
        code!(
            "RFC101",
            Info,
            "rfc4180",
            "LF line endings (RFC 4180 specifies CRLF)",
            "Records are terminated by bare LF. RFC 4180 specifies CRLF, but virtually every \
         consumer accepts LF, so this is informational — relevant only when you must satisfy \
         a validator that enforces the RFC to the letter."
        ),
        code!(
            "RFC102",
            Warning,
            "rfc4180",
            "lone CR line endings",
            "Records are terminated by a bare carriage return (classic Mac OS convention). Many \
         modern parsers do not treat lone CR as a record separator and will read the whole \
         file as one giant record. Convert the line endings to CRLF or LF."
        ),
        code!(
            "RFC103",
            Info,
            "rfc4180",
            "no line break after the last record",
            "The final record is not terminated by a line break. RFC 4180 explicitly permits \
         this, but some stream-processing tools drop or mis-append the last record; add a \
         trailing newline if a downstream tool misbehaves."
        ),
        code!(
            "RFC104",
            Warning,
            "rfc4180",
            "blank line inside the file",
            "An empty line appears between records. Some parsers skip it, others read it as a \
         one-field empty record and fail the column-count check. Delete it."
        ),
        code!(
            "RFC105",
            Info,
            "rfc4180",
            "UTF-8 byte order mark present",
            "The file starts with an EF BB BF byte order mark. Excel needs it to detect UTF-8, \
         but consumers that do not strip it (e.g. Postgres COPY) read it as part of the first \
         field. See PG006 for the Postgres consequence."
        ),
        // --- RFC 4180 records & headers ----------------------------------------
        code!(
            "RFC201",
            Error,
            "rfc4180",
            "field count differs from header",
            "A record has more or fewer fields than the header (or, with --no-header, the first \
         record). This is the single most common cause of bulk-load rejections and is usually \
         a symptom of an unquoted delimiter or an unescaped quote a few fields earlier."
        ),
        code!(
            "RFC202",
            Warning,
            "rfc4180",
            "empty header name",
            "A header field is empty. Consumers that map columns by name either reject the file \
         or invent a placeholder name, and two empty headers collide. Name every column."
        ),
        code!(
            "RFC203",
            Warning,
            "rfc4180",
            "duplicate header name",
            "Two header fields have the same name (byte-for-byte). Column-by-name consumers keep \
         only one of them or fail; BigQuery, pandas and most SQL imports all complain."
        ),
        // --- RFC 4180 encoding ---------------------------------------------------
        code!(
            "RFC301",
            Warning,
            "rfc4180",
            "input is not valid UTF-8",
            "The input contains byte sequences that are not valid UTF-8 — typically Latin-1 or \
         Windows-1252 leftovers. Tolerant consumers show mojibake; strict UTF-8 consumers \
         reject the load (see BQ003/PG003). Re-encode the file as UTF-8."
        ),
        // --- Excel ---------------------------------------------------------------
        code!(
            "XLS001",
            Error,
            "excel",
            "cell exceeds 32,767 characters",
            "Excel's hard per-cell limit is 32,767 characters. On import the cell is truncated \
         (older versions overflow into the next row) with only a soft warning. Split or \
         shorten the value if the file is destined for Excel."
        ),
        code!(
            "XLS002",
            Warning,
            "excel",
            "cell will be interpreted as a formula",
            "The cell starts with =, @, + or -, so Excel evaluates it as a formula instead of \
         text. Besides corrupting data, this is the classic CSV-injection vector (=CMD|...). \
         csvstrict does not flag +/- when the rest parses as a plain number. Prefix the value \
         with a single quote or force the column to text on import."
        ),
        code!(
            "XLS003",
            Warning,
            "excel",
            "file starts with \"ID\" (SYLK misdetection)",
            "A file whose first bytes are the literal characters I and D is detected by Excel as \
         a SYLK spreadsheet, and opening it fails with \"SYLK: File format is not valid\". \
         Rename the column (e.g. lowercase \"id\") or quote it."
        ),
        code!(
            "XLS004",
            Error,
            "excel",
            "more than 16,384 columns",
            "Excel worksheets stop at column XFD (16,384). Extra columns are dropped on import \
         with a one-time warning that is easy to click away."
        ),
        code!(
            "XLS005",
            Error,
            "excel",
            "more than 1,048,576 rows",
            "Excel worksheets stop at row 1,048,576. Everything past that is silently not loaded \
         — the most treacherous way to lose the tail of a dataset."
        ),
        code!(
            "XLS006",
            Info,
            "excel",
            "non-ASCII text without a UTF-8 BOM",
            "The file contains non-ASCII UTF-8 text but no byte order mark. When opened by \
         double-click, Excel decodes it with the system ANSI code page and the text is \
         garbled. Add a BOM, or import via Data → From Text/CSV and pick UTF-8."
        ),
        code!(
            "XLS007",
            Info,
            "excel",
            "leading zeros will be stripped",
            "A digits-only value starts with 0 (postal codes, phone numbers, EAN/UPC). Excel \
         converts it to a number and drops the leading zeros — quoting does NOT prevent \
         this. Import the column as text, or accept the loss."
        ),
        // --- BigQuery ------------------------------------------------------------
        code!(
            "BQ001",
            Warning,
            "bigquery",
            "quoted line break needs allow_quoted_newlines",
            "A quoted field contains a line break. BigQuery rejects the load with \"Missing close \
         double quote (\\\") character\" unless the job sets allow_quoted_newlines=true — and \
         enabling it forces a slower, non-parallel load. Flatten embedded newlines if you can."
        ),
        code!(
            "BQ002",
            Error,
            "bigquery",
            "cell exceeds BigQuery's 100 MB limit",
            "BigQuery's hard maximum for a single CSV cell is 100 MB; the load job fails with \
         \"Row larger than the maximum allowed size\". Split the value or load from a format \
         without the limit."
        ),
        code!(
            "BQ003",
            Error,
            "bigquery",
            "invalid UTF-8 (BigQuery default encoding)",
            "BigQuery decodes CSV as UTF-8 by default; invalid sequences fail the load with \
         \"Bad character (ASCII 0) encountered\" or are replaced, depending on job settings. \
         Re-encode as UTF-8 or pass an explicit --encoding to the load job."
        ),
        code!(
            "BQ004",
            Warning,
            "bigquery",
            "header is not a valid BigQuery column name",
            "With schema auto-detection, BigQuery column names must start with a letter or \
         underscore and contain only letters, digits and underscores (max 300 chars). \
         Offending headers are silently renamed (e.g. \"order id\" → \"order_id\"), which \
         breaks downstream queries that expect the original name."
        ),
        code!("BQ005", Error, "bigquery", "NUL byte (BigQuery rejects the load)",
        "BigQuery fails CSV loads containing 0x00 with \"Bad character (ASCII 0) encountered\". \
         Strip the NULs or re-encode the source (UTF-16 exports are the usual culprit)."),
        // --- Postgres COPY ---------------------------------------------------------
        code!(
            "PG001",
            Error,
            "postgres",
            "NUL byte (COPY rejects it)",
            "Postgres text/varchar values cannot contain 0x00; COPY FROM fails with \"invalid \
         byte sequence for encoding UTF8: 0x00\" at this row. Strip the NULs before loading."
        ),
        code!(
            "PG002",
            Error,
            "postgres",
            "end-of-data marker \\. on its own line",
            "A line containing only \\. is COPY's historical end-of-data marker. Depending on \
         server version and transport, COPY either stops reading here — silently dropping \
         every later row — or errors. Quote the value (\"\\.\") to load it literally."
        ),
        code!(
            "PG003",
            Error,
            "postgres",
            "invalid UTF-8 (server encoding)",
            "Assuming the usual UTF8 server encoding, COPY FROM fails with \"invalid byte \
         sequence for encoding UTF8\" at the first bad byte. Re-encode the file, or declare \
         the true source encoding with COPY ... ENCODING 'LATIN1'."
        ),
        code!(
            "PG004",
            Warning,
            "postgres",
            "header longer than 63 bytes (identifier truncation)",
            "Postgres truncates identifiers to 63 bytes (NAMEDATALEN - 1). If this header is used \
         to create or match a column name it will be silently truncated, and two long headers \
         sharing a 63-byte prefix collide."
        ),
        code!(
            "PG005",
            Info,
            "postgres",
            "mixed empty-field styles (NULL vs empty string)",
            "Under COPY ... WITH (FORMAT csv), an unquoted empty field is NULL while a quoted \
         empty field (\"\") is an empty string. This file mixes both styles, so identical- \
         looking cells load as different values. Pick one convention, or use FORCE_NULL / \
         FORCE_NOT_NULL to override."
        ),
        code!(
            "PG006",
            Warning,
            "postgres",
            "UTF-8 BOM becomes part of the first field",
            "COPY FROM does not strip a UTF-8 byte order mark: the EF BB BF bytes are read as \
         data, so the first header becomes \"\\u{feff}id\" and column matching fails. Remove \
         the BOM before loading into Postgres."
        ),
    ];

/// Look up a code in the registry.
pub fn lookup(code: &str) -> Option<&'static CodeInfo> {
    CODES.iter().find(|c| c.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_unique() {
        for (i, a) in CODES.iter().enumerate() {
            for b in &CODES[i + 1..] {
                assert_ne!(a.code, b.code, "duplicate code {}", a.code);
            }
        }
    }

    #[test]
    fn every_code_belongs_to_a_known_profile() {
        for c in CODES {
            assert!(
                ["rfc4180", "excel", "bigquery", "postgres"].contains(&c.profile),
                "{} has unknown profile {}",
                c.code,
                c.profile
            );
        }
    }

    #[test]
    fn code_prefixes_match_their_profile() {
        for c in CODES {
            let expect = match c.profile {
                "rfc4180" => "RFC",
                "excel" => "XLS",
                "bigquery" => "BQ",
                "postgres" => "PG",
                other => panic!("unknown profile {other}"),
            };
            assert!(c.code.starts_with(expect), "{} vs {}", c.code, c.profile);
        }
    }

    #[test]
    fn lookup_finds_known_and_rejects_unknown() {
        assert_eq!(lookup("PG002").unwrap().profile, "postgres");
        assert!(lookup("RFC999").is_none());
        assert!(lookup("rfc001").is_none(), "lookup is case-sensitive");
    }

    #[test]
    fn titles_and_details_are_nonempty_and_detail_is_longer() {
        for c in CODES {
            assert!(!c.title.is_empty());
            assert!(
                c.detail.len() > c.title.len(),
                "{} detail too short",
                c.code
            );
        }
    }
}
