//! csvstrict — a strict CSV linter.
//!
//! The pipeline is deliberately simple and fully offline:
//!
//! 1. [`scan`] tokenizes raw bytes into records/fields with byte spans and
//!    reports structural RFC 4180 violations (quoting, termination).
//! 2. [`rules`] adds file- and record-level base checks (field counts,
//!    headers, encoding, line endings).
//! 3. The consumer profiles ([`excel`], [`bigquery`], [`postgres`]) layer on
//!    checks for what a *specific* downstream tool will actually reject or
//!    silently mangle.
//! 4. [`report`] renders diagnostics as human-readable text (with a source
//!    snippet and caret) or as JSON, always with byte-precise positions.

pub mod bigquery;
pub mod cli;
pub mod codes;
pub mod diag;
pub mod excel;
pub mod postgres;
pub mod profile;
pub mod report;
pub mod rules;
pub mod scan;

use diag::Diagnostic;
use profile::Profile;

/// Crate version, surfaced by `csvstrict --version` and the JSON reporter.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything the reporters need about one analyzed input.
pub struct Analysis {
    /// All diagnostics, sorted by byte offset (ties broken by code).
    pub diags: Vec<Diagnostic>,
    /// Number of records, including the header record if present.
    pub records: usize,
    /// Total number of fields across all records.
    pub fields: usize,
}

/// Run the full base + profile analysis over a byte buffer.
///
/// `header` controls whether the first record is treated as a header row;
/// `profiles` is the deduplicated list of consumer profiles to apply
/// (the RFC 4180 base checks always run).
pub fn analyze(input: &[u8], delimiter: u8, header: bool, profiles: &[Profile]) -> Analysis {
    let scan = scan::scan(input, delimiter);
    let mut diags = scan.diags.clone();
    rules::check(input, &scan, header, &mut diags);
    for profile in profiles {
        match profile {
            Profile::Rfc4180 => {} // base checks already ran
            Profile::Excel => excel::check(input, &scan, header, &mut diags),
            Profile::BigQuery => bigquery::check(input, &scan, header, &mut diags),
            Profile::Postgres => postgres::check(input, &scan, header, &mut diags),
        }
    }
    diags.sort_by(|a, b| (a.offset, a.code).cmp(&(b.offset, b.code)));
    Analysis {
        diags,
        records: scan.records.len(),
        fields: scan.records.iter().map(|r| r.fields.len()).sum(),
    }
}
