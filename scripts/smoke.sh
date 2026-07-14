#!/usr/bin/env bash
# Smoke test: builds csvstrict and lints the in-repo example files end to
# end, asserting on exit codes, byte-precise positions, profile-specific
# verdicts and both output formats. Self-contained: no network, temp files
# only, finishes in well under a minute. Prints SMOKE OK on success.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

echo "[smoke] building..."
cargo build --quiet
BIN=target/debug/csvstrict

WORK=$(mktemp -d "${TMPDIR:-/tmp}/csvstrict-smoke.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

# --- 1. version/help/profiles sanity -----------------------------------------
"$BIN" --version | grep -q '^csvstrict 0\.1\.0$' || fail "--version mismatch"
"$BIN" --help | grep -q 'USAGE:' || fail "--help missing usage"
"$BIN" profiles | grep -q 'PG002' || fail "profiles must list PG002"
"$BIN" explain XLS003 | grep -q 'SYLK' || fail "explain XLS003 must mention SYLK"
echo "[smoke] meta commands OK"

# --- 2. clean file passes, broken file fails with exact positions ------------
"$BIN" check examples/clean.csv > "$WORK/clean.out" || fail "clean.csv must exit 0"
grep -q ': OK — 4 record(s)' "$WORK/clean.out" || fail "clean summary wrong"

if "$BIN" check examples/broken.csv > "$WORK/broken.out"; then
  fail "broken.csv must exit 1"
fi
grep -q 'broken.csv:3:9: error RFC004' "$WORK/broken.out" || fail "RFC004 position wrong"
grep -q 'broken.csv:4:10: error RFC201' "$WORK/broken.out" || fail "RFC201 position wrong"
grep -q 'broken.csv:6:7: error RFC002' "$WORK/broken.out" || fail "RFC002 position wrong"
grep -q '\^' "$WORK/broken.out" || fail "human output must draw carets"
echo "[smoke] RFC 4180 checks OK (exact line:col anchors)"

# --- 3. profiles change the verdict for the same bytes ------------------------
"$BIN" check examples/bigquery-traps.csv | grep -q ': OK' \
  || fail "bigquery-traps must be clean under the base profile"
"$BIN" check -p bigquery examples/bigquery-traps.csv > "$WORK/bq.out" \
  || fail "bigquery warnings alone must not fail the exit code"
grep -q 'BQ001' "$WORK/bq.out" || fail "quoted newline not flagged for BigQuery"
grep -q 'BQ004' "$WORK/bq.out" || fail "bad column name not flagged for BigQuery"
if "$BIN" check -p bigquery --deny-warnings examples/bigquery-traps.csv > /dev/null; then
  fail "--deny-warnings must turn BigQuery warnings into exit 1"
fi

if "$BIN" check -p postgres examples/postgres-traps.csv > "$WORK/pg.out"; then
  fail "postgres-traps must exit 1 under the postgres profile"
fi
grep -q 'error PG002' "$WORK/pg.out" || fail "\\. end-of-data marker not flagged"

if "$BIN" check -p excel examples/excel-traps.csv > "$WORK/xls.out"; then
  : # warnings only — exit 0 is correct
fi
grep -q 'XLS003' "$WORK/xls.out" || fail "SYLK trap not flagged"
grep -q 'XLS002' "$WORK/xls.out" || fail "formula cell not flagged"
grep -q 'XLS007' "$WORK/xls.out" || fail "leading zeros not flagged"
echo "[smoke] consumer profiles OK (excel / bigquery / postgres)"

# --- 4. JSON output is one machine-readable line per file --------------------
"$BIN" check -f json examples/clean.csv examples/excel-traps.csv -p excel > "$WORK/json.out" \
  || fail "json run must exit 0 (warnings only)"
[ "$(wc -l < "$WORK/json.out")" = 2 ] || fail "expected 2 JSON lines"
grep -q '"code":"XLS003","severity":"warning","profile":"excel","byte":0' "$WORK/json.out" \
  || fail "JSON missing byte-precise XLS003"
grep -q '"summary":{"records":4,"fields":16,"errors":0,"warnings":0,"infos":0}' "$WORK/json.out" \
  || fail "JSON summary for clean.csv wrong"
echo "[smoke] JSON reporter OK"

# --- 5. stdin + custom delimiter ----------------------------------------------
printf 'a;b\r\n1;2;3\r\n' | "$BIN" check -d ';' - > "$WORK/stdin.out" && fail "stdin case must exit 1"
grep -q '<stdin>:2:5: error RFC201' "$WORK/stdin.out" || fail "stdin RFC201 position wrong"
printf 'a;b\r\n1;2\r\n' | "$BIN" check -d ';' - > /dev/null || fail "clean stdin must exit 0"
echo "[smoke] stdin + --delimiter OK"

# --- 6. usage errors exit 2 ----------------------------------------------------
set +e
"$BIN" check /nonexistent/example.test.csv 2> /dev/null; [ $? -eq 2 ] || fail "missing file must exit 2"
"$BIN" check --wat x.csv 2> /dev/null;                   [ $? -eq 2 ] || fail "bad flag must exit 2"
set -e
echo "[smoke] exit codes OK (0 clean / 1 findings / 2 usage)"

echo "SMOKE OK"
