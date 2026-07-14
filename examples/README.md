# csvstrict examples

Small, byte-exact CSV fixtures — one per profile — that the README, the CLI
integration tests and `scripts/smoke.sh` all lint. Every position mentioned
below is stable, so if you edit a fixture, update the tests that pin it.

| File | Try | What it demonstrates |
|---|---|---|
| `clean.csv` | `csvstrict check -p excel,bigquery,postgres examples/clean.csv` | Passes every profile: CRLF endings, quoted delimiter, escaped quote (`""`), trailing newline. |
| `broken.csv` | `csvstrict check examples/broken.csv` | Three structural RFC 4180 errors with exact anchors: a bare quote (3:9), a short record (4:10), an unescaped quote inside a quoted field (6:7) — plus the LF-endings info. |
| `excel-traps.csv` | `csvstrict check -p excel examples/excel-traps.csv` | Legal CSV that Excel mangles: the leading-`ID` SYLK trap, `=SUM(...)` formula injection, leading zeros in zip codes, BOM-less non-ASCII. |
| `bigquery-traps.csv` | `csvstrict check -p bigquery examples/bigquery-traps.csv` | Legal CSV a default `bq load` rejects or rewrites: a quoted newline (needs `allow_quoted_newlines`) and the header `order id` auto-detection renames. |
| `postgres-traps.csv` | `csvstrict check -p postgres examples/postgres-traps.csv` | A `\.` end-of-data line that silently drops the rows after it, and mixed `""`/empty fields that load as different values under COPY. |

Add `-f json` to any command for one machine-readable object per file, or
`-q` for just the verdict line.
