//! Command-line interface: argument parsing (std-only, no clap) and the
//! top-level `run` that maps everything to exit codes.
//!
//! Exit codes: `0` = no errors (warnings/infos allowed unless
//! `--deny-warnings`), `1` = at least one error-level diagnostic,
//! `2` = usage or I/O problem.

use std::io::{Read, Write};

use crate::profile::{parse_list, Profile};
use crate::{codes, report};

const USAGE: &str = "\
csvstrict — strict CSV linter with byte-precise positions

USAGE:
    csvstrict check [OPTIONS] <FILE>...    lint files ('-' reads stdin)
    csvstrict explain <CODE>               explain a diagnostic code (e.g. PG002)
    csvstrict profiles                     list profiles and their checks
    csvstrict --help | --version

OPTIONS (check):
    -p, --profile <LIST>       comma-separated consumer profiles to apply:
                               rfc4180, excel, bigquery, postgres [default: rfc4180]
    -f, --format <FMT>         output format: human, json [default: human]
    -d, --delimiter <CHAR>     single-byte field delimiter; accepts '\\t' [default: ,]
        --no-header            treat the first record as data, not a header
        --max-diagnostics <N>  cap printed diagnostics per file [default: 200]
        --deny-warnings        exit 1 on warnings, not only errors
    -q, --quiet                print only the per-file summary line
";

/// Output format for `check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}

/// Parsed `check` invocation.
#[derive(Debug)]
pub struct CheckOptions {
    pub profiles: Vec<Profile>,
    pub format: Format,
    pub delimiter: u8,
    pub header: bool,
    pub max_diagnostics: usize,
    pub deny_warnings: bool,
    pub quiet: bool,
    pub files: Vec<String>,
}

/// A fully parsed command line.
#[derive(Debug)]
pub enum Command {
    Check(CheckOptions),
    Explain(String),
    Profiles,
    Help,
    Version,
}

/// Parse arguments (without the program name) into a [`Command`].
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter().peekable();
    let Some(first) = it.next() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "-h" | "--help" | "help" => return Ok(Command::Help),
        "-V" | "--version" | "version" => return Ok(Command::Version),
        "profiles" => return Ok(Command::Profiles),
        "explain" => {
            let code = it
                .next()
                .ok_or("explain needs a diagnostic code, e.g. PG002")?;
            return Ok(Command::Explain(code.clone()));
        }
        "check" => {}
        other => return Err(format!("unknown command \"{other}\" (try --help)")),
    }

    let mut opts = CheckOptions {
        profiles: vec![Profile::Rfc4180],
        format: Format::Human,
        delimiter: b',',
        header: true,
        max_diagnostics: 200,
        deny_warnings: false,
        quiet: false,
        files: Vec::new(),
    };
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| -> Result<String, String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-p" | "--profile" => opts.profiles = parse_list(&value("--profile")?)?,
            "-f" | "--format" => {
                opts.format = match value("--format")?.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(format!("unknown format \"{other}\" (human, json)")),
                }
            }
            "-d" | "--delimiter" => opts.delimiter = parse_delimiter(&value("--delimiter")?)?,
            "--no-header" => opts.header = false,
            "--max-diagnostics" => {
                let raw = value("--max-diagnostics")?;
                opts.max_diagnostics =
                    raw.parse::<usize>()
                        .ok()
                        .filter(|&n| n > 0)
                        .ok_or_else(|| {
                            format!("--max-diagnostics needs a positive integer, got \"{raw}\"")
                        })?;
            }
            "--deny-warnings" => opts.deny_warnings = true,
            "-q" | "--quiet" => opts.quiet = true,
            "-" => opts.files.push("-".into()),
            f if f.starts_with('-') => return Err(format!("unknown option \"{f}\" (try --help)")),
            f => opts.files.push(f.into()),
        }
    }
    if opts.files.is_empty() {
        return Err("check needs at least one file (or '-' for stdin)".into());
    }
    Ok(Command::Check(opts))
}

/// Parse the `--delimiter` value: one byte, with `\t` accepted for tabs.
pub fn parse_delimiter(spec: &str) -> Result<u8, String> {
    match spec.as_bytes() {
        [b] => Ok(*b),
        br"\t" => Ok(b'\t'),
        _ => Err(format!(
            "--delimiter must be a single byte (or '\\t'), got \"{spec}\""
        )),
    }
}

/// Execute a parsed command; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match run_to(args, &mut out) {
        Ok(code) => code,
        // A consumer that stops reading early (`csvstrict ... | head`) closes
        // the pipe; treat that as a normal end of output, not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => 0,
        Err(e) => {
            eprintln!("csvstrict: error: {e}");
            2
        }
    }
}

/// Run a parsed command, writing all regular output to `out`.
fn run_to(args: &[String], out: &mut impl Write) -> std::io::Result<i32> {
    let command = match parse_args(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("csvstrict: error: {e}");
            return Ok(2);
        }
    };
    let code = match command {
        Command::Help => {
            write!(out, "{USAGE}")?;
            0
        }
        Command::Version => {
            writeln!(out, "csvstrict {}", crate::VERSION)?;
            0
        }
        Command::Profiles => {
            write!(out, "{}", profiles_text())?;
            0
        }
        Command::Explain(code) => match codes::lookup(&code) {
            Some(info) => {
                writeln!(
                    out,
                    "{} ({}) [{}]: {}\n\n{}",
                    info.code,
                    info.severity.label(),
                    info.profile,
                    info.title,
                    info.detail
                )?;
                0
            }
            None => {
                eprintln!("csvstrict: error: unknown diagnostic code \"{code}\" (see `csvstrict profiles`)");
                2
            }
        },
        Command::Check(opts) => run_check(&opts, out)?,
    };
    out.flush()?;
    Ok(code)
}

/// `csvstrict profiles` output: every profile with its codes.
pub fn profiles_text() -> String {
    let mut out = String::new();
    for p in Profile::ALL {
        out.push_str(&format!("{}  {}\n", p.name(), p.description()));
        for c in codes::CODES.iter().filter(|c| c.profile == p.name()) {
            out.push_str(&format!(
                "    {}  {:<7}  {}\n",
                c.code,
                c.severity.label(),
                c.title
            ));
        }
        out.push('\n');
    }
    out.push_str("Run `csvstrict explain <CODE>` for the full story behind any code.\n");
    out
}

fn run_check(opts: &CheckOptions, out: &mut impl Write) -> std::io::Result<i32> {
    let profile_names: Vec<&str> = std::iter::once("rfc4180") // base checks always apply
        .chain(
            opts.profiles
                .iter()
                .map(|p| p.name())
                .filter(|n| *n != "rfc4180"),
        )
        .collect();
    let mut exit = 0;
    for (i, path) in opts.files.iter().enumerate() {
        let input = match read_input(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("csvstrict: error: {path}: {e}");
                exit = 2;
                continue;
            }
        };
        let label = if path == "-" {
            "<stdin>"
        } else {
            path.as_str()
        };
        let analysis = crate::analyze(&input, opts.delimiter, opts.header, &opts.profiles);
        match opts.format {
            Format::Human => {
                if i > 0 && !opts.quiet {
                    writeln!(out)?;
                }
                write!(
                    out,
                    "{}",
                    report::render_human(
                        label,
                        &input,
                        &analysis,
                        &profile_names,
                        opts.max_diagnostics,
                        opts.quiet
                    )
                )?;
            }
            Format::Json => writeln!(
                out,
                "{}",
                report::render_json(label, &input, &analysis, &profile_names)
            )?,
        }
        let t = report::totals(&analysis.diags);
        let failing = t.errors > 0 || (opts.deny_warnings && t.warnings > 0);
        if failing && exit == 0 {
            exit = 1;
        }
    }
    Ok(exit)
}

fn read_input(path: &str) -> std::io::Result<Vec<u8>> {
    if path == "-" {
        let mut buf = Vec::new();
        std::io::stdin().lock().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn check_defaults_are_sane() {
        let Command::Check(o) = parse(&["check", "a.csv"]).unwrap() else {
            panic!()
        };
        assert_eq!(o.profiles, vec![Profile::Rfc4180]);
        assert_eq!(o.format, Format::Human);
        assert_eq!(o.delimiter, b',');
        assert!(o.header);
        assert_eq!(o.max_diagnostics, 200);
        assert!(!o.deny_warnings && !o.quiet);
        assert_eq!(o.files, vec!["a.csv"]);
    }

    #[test]
    fn all_check_flags_parse() {
        let Command::Check(o) = parse(&[
            "check",
            "-p",
            "excel,bq",
            "-f",
            "json",
            "-d",
            ";",
            "--no-header",
            "--max-diagnostics",
            "5",
            "--deny-warnings",
            "-q",
            "a.csv",
            "-",
        ])
        .unwrap() else {
            panic!()
        };
        assert_eq!(o.profiles, vec![Profile::Excel, Profile::BigQuery]);
        assert_eq!(o.format, Format::Json);
        assert_eq!(o.delimiter, b';');
        assert!(!o.header);
        assert_eq!(o.max_diagnostics, 5);
        assert!(o.deny_warnings && o.quiet);
        assert_eq!(o.files, vec!["a.csv", "-"]);
    }

    #[test]
    fn delimiter_accepts_tab_escape_and_rejects_multibyte() {
        assert_eq!(parse_delimiter("\\t").unwrap(), b'\t');
        assert_eq!(parse_delimiter("\t").unwrap(), b'\t');
        assert_eq!(parse_delimiter("|").unwrap(), b'|');
        assert!(parse_delimiter(";;").is_err());
        assert!(parse_delimiter("").is_err());
    }

    #[test]
    fn usage_errors_are_reported_not_panicked() {
        assert!(parse(&["check"]).is_err(), "no files");
        assert!(parse(&["check", "--format", "xml", "a.csv"]).is_err());
        assert!(parse(&["check", "--wat", "a.csv"]).is_err());
        assert!(parse(&["check", "--max-diagnostics", "0", "a.csv"]).is_err());
        assert!(parse(&["frobnicate"]).is_err());
        assert!(parse(&["explain"]).is_err());
    }

    #[test]
    fn top_level_commands_parse() {
        assert!(matches!(parse(&["--help"]).unwrap(), Command::Help));
        assert!(matches!(parse(&[]).unwrap(), Command::Help));
        assert!(matches!(parse(&["--version"]).unwrap(), Command::Version));
        assert!(matches!(parse(&["profiles"]).unwrap(), Command::Profiles));
        let Command::Explain(code) = parse(&["explain", "PG002"]).unwrap() else {
            panic!()
        };
        assert_eq!(code, "PG002");
    }

    #[test]
    fn run_to_writes_to_the_given_writer_and_maps_exit_codes() {
        let mut buf = Vec::new();
        let code = run_to(&["--version".to_string()], &mut buf).unwrap();
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            format!("csvstrict {}\n", crate::VERSION)
        );
        // Usage errors go to stderr and surface as exit code 2, not as Err —
        // Err is reserved for I/O failures on `out` (e.g. a closed pipe).
        let mut empty = Vec::new();
        let code = run_to(&["frobnicate".to_string()], &mut empty).unwrap();
        assert_eq!((code, empty.len()), (2, 0));
    }

    #[test]
    fn profiles_text_lists_every_registered_code() {
        let text = profiles_text();
        for c in codes::CODES {
            assert!(text.contains(c.code), "profiles output missing {}", c.code);
        }
        for p in Profile::ALL {
            assert!(text.contains(p.name()));
        }
    }
}
