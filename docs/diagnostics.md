# Diagnostic reference

Every code csvstrict can emit, grouped by profile. This table is generated
from the single registry in `src/codes.rs`; `csvstrict explain <CODE>` prints
the same text at the terminal, and `csvstrict profiles` prints the short form.

Severities: **error** fails the exit code (1), **warning** fails only with
`--deny-warnings`, **info** never fails.

## rfc4180 (base — always applied)

| Code | Severity | Meaning |
|---|---|---|
| RFC001 | error | Unterminated quoted field. Reported at the *opening* quote, because everything after it was swallowed into the field — which is why other tools blame the last line instead. |
| RFC002 | error | Unescaped `"` inside a quoted field. Write `""` for a literal quote. csvstrict recovers by treating the quote as data, so the rest of the record still parses. |
| RFC003 | warning | Whitespace between a closing quote and the delimiter. Strict parsers reject the padding; lenient ones glue it onto the value. |
| RFC004 | error | Bare `"` in an unquoted field. Includes a targeted hint when the cause is a space before the opening quote (which makes the whole field unquoted). |
| RFC005 | error | NUL byte (0x00). Usually a UTF-16 export; nothing mainstream accepts it. |
| RFC101 | info | Records end with bare LF; RFC 4180 specifies CRLF. Reported once with a count. |
| RFC102 | warning | Records end with a lone CR (classic Mac). Many parsers read the whole file as one line. |
| RFC103 | info | No line break after the last record (permitted by the RFC; some tools mind). |
| RFC104 | warning | Blank line between records, read as a one-field empty record. |
| RFC105 | info | UTF-8 BOM present. Good for Excel, bad for Postgres (see PG006). |
| RFC201 | error | Field count differs from the header. Points at the first surplus field, or at the exact byte where the missing field would start. |
| RFC202 | warning | Empty header name. |
| RFC203 | warning | Duplicate header name (byte-for-byte, after unquoting). |
| RFC301 | warning | Invalid UTF-8, with the first bad byte's offset and a total count. Escalates to an error under the bigquery/postgres profiles. |

## excel

| Code | Severity | Meaning |
|---|---|---|
| XLS001 | error | Cell longer than 32,767 characters — Excel truncates it on import. Measured in characters, not bytes. |
| XLS002 | warning | Cell starts with `=`, `@`, `+` or `-` and is not a plain number: Excel evaluates it as a formula (the CSV-injection vector). |
| XLS003 | warning | File starts with the bytes `ID`: Excel misdetects SYLK and refuses to open the file. Quoting or a BOM defuses it. |
| XLS004 | error | More than 16,384 columns (past column XFD): the rest are dropped. |
| XLS005 | error | More than 1,048,576 rows: the tail is silently not loaded. Points at the first lost record. |
| XLS006 | info | Non-ASCII UTF-8 without a BOM: double-click import decodes with the ANSI code page (mojibake). |
| XLS007 | info | Digits-only value with leading zeros: Excel converts to a number and drops them — even when quoted. |

## bigquery

| Code | Severity | Meaning |
|---|---|---|
| BQ001 | warning | Line break inside a quoted field: the load fails unless `allow_quoted_newlines=true`, which in turn disables parallel loading. Points at the embedded newline byte. |
| BQ002 | error | Cell larger than 100 MB, BigQuery's hard per-cell limit for CSV. |
| BQ003 | error | Invalid UTF-8 under the default encoding: the load job fails. |
| BQ004 | warning | Header is not a valid BigQuery column name (`[A-Za-z_][A-Za-z0-9_]*`, ≤ 300 chars): schema auto-detection silently renames it. |
| BQ005 | error | NUL byte: `Bad character (ASCII 0) encountered`. |

## postgres (COPY ... WITH (FORMAT csv))

| Code | Severity | Meaning |
|---|---|---|
| PG001 | error | NUL byte: `invalid byte sequence for encoding UTF8: 0x00`. |
| PG002 | error | A line containing only `\.` — COPY's end-of-data marker. The message counts how many records after it would be dropped. |
| PG003 | error | Invalid UTF-8 under a UTF8 server encoding: COPY aborts at this byte. |
| PG004 | warning | Header longer than 63 bytes: truncated as an identifier (NAMEDATALEN − 1), measured in bytes. |
| PG005 | info | File mixes unquoted empty fields (NULL) and quoted empty fields (empty string): identical-looking cells load as different values. |
| PG006 | warning | UTF-8 BOM: COPY reads it as data, so it becomes part of the first header and column matching fails. |

## Adding a code

1. Add the row to `src/codes.rs` (code, fixed severity, owning profile, title, detail). The prefix must match the profile (`RFC`/`XLS`/`BQ`/`PG`) — a unit test enforces this.
2. Emit it from the relevant module with `Diagnostic::new("...", offset, message)`, attaching `.span()`, `.record()` and `.field()` where they are known.
3. Add the row to this file, and tests that pin the exact offset on a minimal input.
