# Contributing to csvstrict

Thanks for your interest in improving csvstrict. Issues, discussions and pull requests are all welcome.

## Getting started

Prerequisites: Rust 1.75 or newer (stable toolchain).

```bash
git clone https://github.com/JaydenCJ/csvstrict.git
cd csvstrict
cargo build
cargo test
bash scripts/smoke.sh
```

`scripts/smoke.sh` lints the in-repo example files end to end and asserts on exit codes, exact `line:col` positions and both output formats. It finishes in under a minute and must print `SMOKE OK`.

## Before you open a pull request

1. `cargo fmt` — formatting is enforced.
2. `cargo clippy --all-targets -- -D warnings` — clippy must be clean.
3. `cargo test` — unit tests and the CLI integration tests must pass.
4. `bash scripts/smoke.sh` — the smoke test must print `SMOKE OK`.
5. Add tests for behavior changes. All linting logic lives in pure modules (`scan`, `rules`, `excel`, `bigquery`, `postgres`) that take bytes in and push diagnostics out; please keep it that way.

## Ground rules

- Keep dependencies at zero. csvstrict is implemented on std alone; adding a dependency needs a very strong justification in the PR description.
- No network calls, no telemetry — csvstrict only ever reads the files you pass it.
- Every new diagnostic needs a row in `src/codes.rs` (the registry drives `explain`, `profiles` and the docs) plus an entry in `docs/diagnostics.md`.
- Profile checks must reflect what the real consumer actually does, with the concrete error message or silent behavior named in the code's detail text. No speculative rules.
- Code comments and doc comments are written in English.

## Reporting bugs

Please include the `csvstrict --version` output, the exact command line, and a minimal CSV that reproduces the problem (a hex dump of the relevant bytes helps for quoting/encoding issues, e.g. `xxd -s <byte> -l 32 file.csv` around the reported offset). If csvstrict's verdict disagrees with a real consumer (Excel, BigQuery, Postgres), the consumer's exact error message is the most valuable piece of evidence.

## Security

If you find a security issue (e.g. a panic on adversarial input that could take down a pipeline), please do not open a public issue. Use GitHub's private vulnerability reporting on this repository instead.
