//! Consumer profiles: named sets of checks describing what a specific
//! downstream tool rejects or silently mangles.

/// A consumer profile selectable with `--profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The RFC 4180 base checks (always on; selecting it adds nothing).
    Rfc4180,
    /// Microsoft Excel import behavior (limits, SYLK, formulas, encoding).
    Excel,
    /// BigQuery CSV load jobs (encoding, quoted newlines, naming, limits).
    BigQuery,
    /// Postgres `COPY ... WITH (FORMAT csv)` (NULs, `\.`, identifiers, BOM).
    Postgres,
}

impl Profile {
    /// All profiles, in the order `csvstrict profiles` lists them.
    pub const ALL: [Profile; 4] = [
        Profile::Rfc4180,
        Profile::Excel,
        Profile::BigQuery,
        Profile::Postgres,
    ];

    /// Canonical lowercase name, matching the `profile` field on diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Rfc4180 => "rfc4180",
            Profile::Excel => "excel",
            Profile::BigQuery => "bigquery",
            Profile::Postgres => "postgres",
        }
    }

    /// One-line description for `csvstrict profiles`.
    pub fn description(self) -> &'static str {
        match self {
            Profile::Rfc4180 => "RFC 4180 structure, headers, encoding (always applied)",
            Profile::Excel => {
                "Microsoft Excel: cell/row/column limits, SYLK trap, formula injection, encoding"
            }
            Profile::BigQuery => {
                "BigQuery CSV load jobs: UTF-8, quoted newlines, column names, size limits"
            }
            Profile::Postgres => {
                "Postgres COPY FROM (FORMAT csv): NUL bytes, \\. marker, identifiers, BOM"
            }
        }
    }

    /// Parse a user-supplied profile name (accepts common aliases).
    pub fn parse(name: &str) -> Option<Profile> {
        match name.to_ascii_lowercase().as_str() {
            "rfc4180" | "rfc" | "base" => Some(Profile::Rfc4180),
            "excel" | "xls" | "xlsx" => Some(Profile::Excel),
            "bigquery" | "bq" => Some(Profile::BigQuery),
            "postgres" | "postgresql" | "pg" | "copy" => Some(Profile::Postgres),
            _ => None,
        }
    }
}

/// Parse a comma-separated `--profile` value into a deduplicated list.
pub fn parse_list(spec: &str) -> Result<Vec<Profile>, String> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let profile = Profile::parse(part).ok_or_else(|| {
            format!(
                "unknown profile \"{part}\" (expected one of: rfc4180, excel, bigquery, postgres)"
            )
        })?;
        if !out.contains(&profile) {
            out.push(profile);
        }
    }
    if out.is_empty() {
        return Err("empty --profile value".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_canonical_names_and_aliases() {
        assert_eq!(Profile::parse("excel"), Some(Profile::Excel));
        assert_eq!(Profile::parse("BQ"), Some(Profile::BigQuery));
        assert_eq!(Profile::parse("postgresql"), Some(Profile::Postgres));
        assert_eq!(Profile::parse("copy"), Some(Profile::Postgres));
        assert_eq!(Profile::parse("rfc"), Some(Profile::Rfc4180));
        assert_eq!(Profile::parse("parquet"), None);
    }

    #[test]
    fn parse_list_splits_trims_and_dedupes() {
        let p = parse_list("excel, bq,excel").unwrap();
        assert_eq!(p, vec![Profile::Excel, Profile::BigQuery]);
    }

    #[test]
    fn parse_list_rejects_unknown_and_empty() {
        assert!(parse_list("excel,nope").unwrap_err().contains("nope"));
        assert!(parse_list(" , ").is_err());
    }

    #[test]
    fn names_round_trip_through_parse() {
        for p in Profile::ALL {
            assert_eq!(Profile::parse(p.name()), Some(p));
        }
    }
}
