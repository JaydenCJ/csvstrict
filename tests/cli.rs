//! End-to-end tests against the compiled `csvstrict` binary: exit codes,
//! byte-precise positions in both output formats, profile selection, stdin,
//! and the meta commands. Everything runs on in-repo fixtures and temp files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_csvstrict")
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to run csvstrict")
}

fn run_stdin(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn csvstrict");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn example(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
        .display()
        .to_string()
}

fn tempfile(tag: &str, contents: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("csvstrict-cli-{tag}-{}.csv", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn meta_commands_version_help_profiles_explain() {
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "csvstrict 0.1.0"
    );

    let help = run(&["--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout).to_string();
    for needle in [
        "check",
        "explain",
        "profiles",
        "--profile",
        "--format",
        "--deny-warnings",
    ] {
        assert!(text.contains(needle), "help must mention {needle}");
    }

    let profiles = run(&["profiles"]);
    assert!(profiles.status.success());
    let text = String::from_utf8_lossy(&profiles.stdout).to_string();
    for needle in [
        "rfc4180", "excel", "bigquery", "postgres", "RFC001", "XLS003", "BQ001", "PG002",
    ] {
        assert!(text.contains(needle), "profiles must list {needle}");
    }

    let explain = run(&["explain", "XLS003"]);
    assert!(explain.status.success());
    assert!(String::from_utf8_lossy(&explain.stdout).contains("SYLK"));

    let unknown = run(&["explain", "RFC999"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("RFC999"));
}

#[test]
fn clean_file_exits_zero_and_broken_file_reports_exact_positions() {
    let clean = run(&["check", &example("clean.csv")]);
    assert_eq!(clean.status.code(), Some(0), "{clean:?}");
    assert!(String::from_utf8_lossy(&clean.stdout).contains(": OK — 4 record(s)"));

    let broken = run(&["check", &example("broken.csv")]);
    assert_eq!(broken.status.code(), Some(1));
    let text = String::from_utf8_lossy(&broken.stdout).to_string();
    // Exact line:col anchors for the three structural defects in the fixture.
    assert!(text.contains("broken.csv:3:9: error RFC004"), "{text}");
    assert!(text.contains("broken.csv:4:10: error RFC201"), "{text}");
    assert!(text.contains("broken.csv:6:7: error RFC002"), "{text}");
    assert!(text.contains("3 error(s)"), "{text}");
    assert!(text.contains('^'), "human output must draw carets");
}

#[test]
fn json_format_emits_one_machine_readable_line_per_file() {
    let out = run(&[
        "check",
        "-f",
        "json",
        &example("broken.csv"),
        &example("clean.csv"),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "broken input keeps the failing exit code"
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "one JSON object per input file");
    assert!(lines[0].starts_with('{') && lines[0].ends_with('}'));
    // RFC004: bare quote at byte 51 of the fixture (line 3, col 9).
    assert!(
        lines[0].contains(r#""code":"RFC004","severity":"error","profile":"rfc4180","byte":51,"len":1,"line":3,"col":9"#),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].contains(r#""errors":0,"warnings":0,"infos":0"#),
        "{}",
        lines[1]
    );
}

#[test]
fn profiles_change_the_verdict_for_the_same_bytes() {
    // Base RFC checks: quoted newlines and "order id" are perfectly legal.
    let base = run(&["check", &example("bigquery-traps.csv")]);
    assert_eq!(base.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&base.stdout).contains(": OK"));

    // The BigQuery profile turns them into warnings...
    let bq = run(&["check", "-p", "bigquery", &example("bigquery-traps.csv")]);
    assert_eq!(bq.status.code(), Some(0));
    let text = String::from_utf8_lossy(&bq.stdout).to_string();
    assert!(text.contains("BQ004") && text.contains("BQ001"), "{text}");

    // ...and --deny-warnings turns the warnings into a failing exit code.
    let strict = run(&[
        "check",
        "-p",
        "bigquery",
        "--deny-warnings",
        &example("bigquery-traps.csv"),
    ]);
    assert_eq!(strict.status.code(), Some(1));

    // Postgres profile: \. end-of-data marker is an error with a byte offset.
    let pg = run(&[
        "check",
        "-p",
        "postgres",
        "-q",
        &example("postgres-traps.csv"),
    ]);
    assert_eq!(pg.status.code(), Some(1));
    let quiet = String::from_utf8_lossy(&pg.stdout).to_string();
    assert_eq!(
        quiet.lines().count(),
        1,
        "--quiet prints only the summary: {quiet}"
    );
    assert!(quiet.contains("2 error(s)"), "{quiet}");
}

#[test]
fn closed_stdout_pipe_ends_the_run_quietly() {
    // `csvstrict check noisy.csv | head` closes the pipe long before the
    // report is fully written. The failed write must end the run cleanly —
    // not panic — or every pipeline that stops reading early breaks.
    let mut noisy = Vec::from(&b"h\n"[..]);
    for _ in 0..20_000 {
        noisy.extend_from_slice(b"=formula\n"); // one XLS002 warning per row
    }
    let path = tempfile("pipe", &noisy);
    let mut child = Command::new(bin())
        .args([
            "check",
            "-p",
            "excel",
            "--max-diagnostics",
            "20000",
            path.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn csvstrict");
    drop(child.stdout.take()); // close the read end without reading anything
    let out = child.wait_with_output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "clean exit, got {:?}",
        out.status
    );
    assert!(
        out.stderr.is_empty(),
        "nothing on stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn stdin_custom_delimiter_and_usage_errors() {
    // Stdin with a semicolon delimiter: field counts are checked against it.
    let out = run_stdin(&["check", "-d", ";", "-"], b"a;b\r\n1;2;3\r\n");
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("<stdin>:2:5: error RFC201"), "{text}");

    // The same bytes are clean when the delimiter matches the data.
    let ok = run_stdin(&["check", "-d", ";", "-"], b"a;b\r\n1;2\r\n");
    assert_eq!(ok.status.code(), Some(0));

    // Usage problems exit 2 and never panic.
    let nofile = run(&["check"]);
    assert_eq!(nofile.status.code(), Some(2));
    let badflag = run(&["check", "--wat", "x.csv"]);
    assert_eq!(badflag.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&badflag.stderr).contains("--wat"));
    let missing = run(&["check", "/nonexistent/example.test.csv"]);
    assert_eq!(missing.status.code(), Some(2));

    // --max-diagnostics truncates output but keeps true totals.
    let noisy = tempfile("noisy", b"h\n=1\n=2\n=3\n");
    let out = run(&[
        "check",
        "-p",
        "excel",
        "--max-diagnostics",
        "1",
        noisy.to_str().unwrap(),
    ]);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("truncated at 1 of"), "{text}");
    assert!(text.contains("3 warning(s)"), "totals stay exact: {text}");
    std::fs::remove_file(noisy).unwrap();
}
