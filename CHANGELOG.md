# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-13

### Added

- Byte-level RFC 4180 scanner with error recovery: unterminated quotes (RFC001), unescaped quotes inside quoted fields (RFC002), padding after closing quotes (RFC003), bare quotes in unquoted fields (RFC004) and NUL bytes (RFC005) are each reported at the exact offending byte, and one missing quote produces one diagnostic instead of cascading to end of file.
- Base file-shape and consistency checks: field-count mismatches pointing at the first surplus field or the missing field's position (RFC201), blank lines (RFC104), empty and duplicate headers (RFC202/RFC203), LF/CR line-ending conventions (RFC101/RFC102), missing final newline (RFC103), UTF-8 BOM (RFC105) and invalid UTF-8 with per-sequence offsets (RFC301).
- Excel profile: 32,767-character cell truncation (XLS001), formula interpretation / CSV injection with a numeric +/- exemption (XLS002), the "ID" SYLK misdetection trap (XLS003), the 16,384-column (XLS004) and 1,048,576-row (XLS005) worksheet limits, BOM-less non-ASCII mojibake (XLS006) and leading-zero stripping (XLS007).
- BigQuery profile: quoted newlines that require `allow_quoted_newlines` (BQ001), the 100 MB cell limit (BQ002), invalid UTF-8 (BQ003), column names schema auto-detection will rename (BQ004) and NUL bytes (BQ005).
- Postgres COPY profile: NUL bytes (PG001), the `\.` end-of-data marker with a count of the records it would drop (PG002), invalid UTF-8 (PG003), 63-byte identifier truncation (PG004), mixed NULL/empty-string conventions (PG005) and the BOM-in-first-column trap (PG006).
- CLI: `check` with `--profile`, `--format human|json`, `--delimiter` (including `\t`), `--no-header`, `--max-diagnostics`, `--deny-warnings`, `--quiet` and stdin via `-`; `explain <CODE>` and `profiles` backed by a single code registry; exit codes 0/1/2.
- Human reporter with `path:line:col` anchors, source snippets, character-aligned caret spans (multibyte-safe) and windowing for long lines; JSON reporter emitting one machine-readable object per input with byte, line, column, record and field for every diagnostic.
- Example CSV fixtures for every profile under `examples/`, a full code reference in `docs/diagnostics.md`.
- Test suite: 87 unit tests, 6 CLI integration tests, and `scripts/smoke.sh`.

[0.1.0]: https://github.com/JaydenCJ/csvstrict/releases/tag/v0.1.0
