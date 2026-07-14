//! Reporters: human-readable text with source snippets and carets, and
//! machine-readable JSON (one object per input). Both always carry the
//! byte offset alongside line/column.

use crate::diag::{Diagnostic, LineIndex, Severity};
use crate::Analysis;

/// Widest snippet (in characters) shown under a diagnostic before windowing.
const SNIPPET_WIDTH: usize = 120;

/// Per-severity totals for one input.
#[derive(Default)]
pub struct Totals {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

pub fn totals(diags: &[Diagnostic]) -> Totals {
    let mut t = Totals::default();
    for d in diags {
        match d.severity {
            Severity::Error => t.errors += 1,
            Severity::Warning => t.warnings += 1,
            Severity::Info => t.infos += 1,
        }
    }
    t
}

/// Render the human report for one input into a string.
///
/// `max` caps the number of diagnostics printed (never the counts); `quiet`
/// suppresses everything except the summary line.
pub fn render_human(
    path: &str,
    input: &[u8],
    analysis: &Analysis,
    profiles: &[&str],
    max: usize,
    quiet: bool,
) -> String {
    let index = LineIndex::new(input);
    let mut out = String::new();
    if !quiet {
        for d in analysis.diags.iter().take(max) {
            render_one(&mut out, path, input, &index, d);
        }
        if analysis.diags.len() > max {
            out.push_str(&format!(
                "... output truncated at {max} of {} diagnostics; raise with --max-diagnostics\n\n",
                analysis.diags.len()
            ));
        }
    }
    out.push_str(&summary_line(path, analysis, profiles));
    out.push('\n');
    out
}

fn render_one(out: &mut String, path: &str, input: &[u8], index: &LineIndex, d: &Diagnostic) {
    let (line, col) = index.position(d.offset);
    out.push_str(&format!(
        "{path}:{line}:{col}: {} {} [{}]: {}",
        d.severity.label(),
        d.code,
        d.profile,
        d.message
    ));
    match (d.record, d.field) {
        (Some(r), Some(f)) => out.push_str(&format!(" (record {r}, field {f})")),
        (Some(r), None) => out.push_str(&format!(" (record {r})")),
        _ => {}
    }
    out.push('\n');
    render_snippet(out, input, index, d, line);
    out.push('\n');
}

/// Print the offending line with a caret run under the exact bytes,
/// windowed around the caret when the line is long.
fn render_snippet(out: &mut String, input: &[u8], index: &LineIndex, d: &Diagnostic, line: usize) {
    let (start, end) = index.line_span(line, input);
    if start >= end && d.offset >= end {
        return; // empty line (e.g. blank-line diagnostics): nothing to show
    }
    // Decode the line lossily and remember, per character, whether the caret
    // span covers its originating bytes. Control characters render as '·' so
    // the caret stays aligned.
    let line_bytes = &input[start..end];
    let mut chars: Vec<(char, bool)> = Vec::new();
    let mut caret_from = None;
    let mut byte = start;
    for chunk in String::from_utf8_lossy(line_bytes).chars() {
        let width = chunk.len_utf8().max(1);
        let covered = byte < d.offset + d.len && byte + width > d.offset;
        if covered && caret_from.is_none() {
            caret_from = Some(chars.len());
        }
        let shown = if chunk.is_control() {
            '\u{00B7}'
        } else {
            chunk
        };
        chars.push((shown, covered));
        byte += width;
    }
    // Diagnostics pointing at the terminator or EOF sit one past the text.
    if caret_from.is_none() {
        chars.push((' ', true));
        caret_from = Some(chars.len() - 1);
    }
    let caret_at = caret_from.unwrap_or(0);
    // Window long lines around the caret.
    let (win_start, prefix) = if chars.len() <= SNIPPET_WIDTH {
        (0, "")
    } else {
        (caret_at.saturating_sub(SNIPPET_WIDTH / 2), "...")
    };
    let windowed = &chars[win_start..(win_start + SNIPPET_WIDTH).min(chars.len())];
    let suffix = if win_start + SNIPPET_WIDTH < chars.len() {
        "..."
    } else {
        ""
    };

    // The phantom character for end-of-line diagnostics would print as a
    // trailing space; trim it (the caret line below still marks the column).
    let mut src = String::new();
    src.push_str(prefix);
    for (c, _) in windowed {
        src.push(*c);
    }
    src.push_str(suffix);
    let gutter = format!("{line:>6} | ");
    out.push_str(&gutter);
    out.push_str(src.trim_end());
    out.push('\n');
    out.push_str(&" ".repeat(gutter.len() - 2));
    out.push_str("| ");
    out.push_str(&" ".repeat(prefix.len()));
    for (_, covered) in windowed {
        out.push(if *covered { '^' } else { ' ' });
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out.push('\n');
}

/// One-line per-file summary, also used by `--quiet`.
pub fn summary_line(path: &str, analysis: &Analysis, profiles: &[&str]) -> String {
    let t = totals(&analysis.diags);
    let verdict = if t.errors == 0 && t.warnings == 0 && t.infos == 0 {
        "OK".to_string()
    } else {
        format!(
            "{} error(s), {} warning(s), {} info(s)",
            t.errors, t.warnings, t.infos
        )
    };
    format!(
        "{path}: {verdict} — {} record(s), {} field(s) checked [profiles: {}]",
        analysis.records,
        analysis.fields,
        profiles.join(", ")
    )
}

/// Render one input as a single-line JSON object (JSONL across files).
pub fn render_json(path: &str, input: &[u8], analysis: &Analysis, profiles: &[&str]) -> String {
    let index = LineIndex::new(input);
    let t = totals(&analysis.diags);
    let mut out = String::from("{");
    out.push_str(&format!(
        "\"tool\":\"csvstrict\",\"version\":\"{}\",",
        crate::VERSION
    ));
    out.push_str(&format!("\"path\":{},", json_string(path)));
    out.push_str("\"profiles\":[");
    for (i, p) in profiles.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&json_string(p));
    }
    out.push_str("],");
    out.push_str(&format!(
        "\"summary\":{{\"records\":{},\"fields\":{},\"errors\":{},\"warnings\":{},\"infos\":{}}},",
        analysis.records, analysis.fields, t.errors, t.warnings, t.infos
    ));
    out.push_str("\"diagnostics\":[");
    for (i, d) in analysis.diags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let (line, col) = index.position(d.offset);
        out.push_str(&format!(
            "{{\"code\":\"{}\",\"severity\":\"{}\",\"profile\":\"{}\",\"byte\":{},\"len\":{},\
             \"line\":{line},\"col\":{col},\"record\":{},\"field\":{},\"message\":{}}}",
            d.code,
            d.severity.label(),
            d.profile,
            d.offset,
            d.len,
            d.record.map_or("null".to_string(), |r| r.to_string()),
            d.field.map_or("null".to_string(), |f| f.to_string()),
            json_string(&d.message)
        ));
    }
    out.push_str("]}");
    out
}

/// Minimal JSON string encoder (RFC 8259 escaping).
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;

    fn analyze(input: &[u8]) -> Analysis {
        crate::analyze(input, b',', true, &[Profile::Rfc4180])
    }

    #[test]
    fn human_report_pins_the_caret_under_the_offending_byte() {
        let input = b"a,b,c\r\n1,2\r\n";
        let a = analyze(input);
        let text = render_human("t.csv", input, &a, &["rfc4180"], 100, false);
        assert!(text.contains("t.csv:2:4: error RFC201"), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        let src = lines
            .iter()
            .position(|l| l.contains("| 1,2"))
            .expect("snippet line");
        let caret_line = lines[src + 1];
        let src_col = lines[src].find("1,2").unwrap();
        assert_eq!(
            caret_line.find('^').unwrap(),
            src_col + 3,
            "caret one past \"1,2\""
        );
    }

    #[test]
    fn caret_spans_multibyte_text_by_characters() {
        // The duplicate header "aéb" is 4 bytes but 3 chars; the caret run
        // under it must be 3 characters wide to line up visually.
        let input: &[u8] = b"a\xC3\xA9b,a\xC3\xA9b\r\n1,2\r\n";
        let a = crate::analyze(input, b',', true, &[Profile::Rfc4180]);
        let text = render_human("t.csv", input, &a, &["rfc4180"], 100, false);
        let lines: Vec<&str> = text.lines().collect();
        let caret = lines
            .iter()
            .find(|l| l.trim_start().starts_with('|') && l.contains('^'))
            .expect("caret line");
        assert_eq!(caret.matches('^').count(), 3, "{text}");
    }

    #[test]
    fn truncation_note_appears_when_diagnostics_exceed_max() {
        let input = b"a\r\n=1\r\n=2\r\n=3\r\n";
        let a = crate::analyze(input, b',', true, &[Profile::Excel]);
        assert!(a.diags.len() >= 3);
        let text = render_human("t.csv", input, &a, &["excel"], 1, false);
        assert!(text.contains("truncated at 1 of"), "{text}");
    }

    #[test]
    fn snippet_lines_carry_no_trailing_whitespace() {
        // The RFC201 for the short record points one past the line's text;
        // the phantom column must not leave a trailing space on the source
        // line (it would break byte-exact comparisons of captured output).
        let input = b"a,b,c\r\n1,2\r\n";
        let a = analyze(input);
        let text = render_human("t.csv", input, &a, &["rfc4180"], 100, false);
        for line in text.lines() {
            assert_eq!(line, line.trim_end(), "trailing whitespace in {line:?}");
        }
    }

    #[test]
    fn quiet_mode_prints_only_the_summary() {
        let input = b"a,b\r\n1\r\n";
        let a = analyze(input);
        let text = render_human("t.csv", input, &a, &["rfc4180"], 100, true);
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("1 error(s)"));
    }

    #[test]
    fn clean_file_summary_says_ok() {
        let input = b"a,b\r\n1,2\r\n";
        let a = analyze(input);
        let line = summary_line("clean.csv", &a, &["rfc4180"]);
        assert_eq!(
            line,
            "clean.csv: OK — 2 record(s), 4 field(s) checked [profiles: rfc4180]"
        );
    }

    #[test]
    fn json_report_is_single_line_with_byte_and_position() {
        let input = b"a,b\r\n1\r\n";
        let a = analyze(input);
        let json = render_json("t.csv", input, &a, &["rfc4180"]);
        assert_eq!(json.lines().count(), 1);
        assert!(json.contains("\"code\":\"RFC201\""));
        assert!(json.contains("\"byte\":6"));
        assert!(json.contains("\"line\":2,\"col\":2"));
        assert!(json.contains("\"errors\":1"));
        assert!(json.starts_with('{') && json.ends_with('}'));
    }

    #[test]
    fn json_string_escapes_quotes_backslashes_and_controls() {
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("a\nb\tc"), "\"a\\nb\\tc\"");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
        assert_eq!(json_string("héllo"), "\"héllo\"");
    }

    #[test]
    fn long_lines_are_windowed_around_the_caret() {
        let mut input = b"h\r\n".to_vec();
        input.extend(std::iter::repeat(b'x').take(400));
        input.extend(b"\"\r\n"); // bare quote at the end of a long line
        let a = analyze(&input);
        let text = render_human("t.csv", &input, &a, &["rfc4180"], 100, false);
        let snippet = text
            .lines()
            .find(|l| l.contains("..."))
            .expect("windowed snippet");
        assert!(snippet.len() < 400, "snippet must be windowed: {snippet}");
        assert!(text.contains('^'));
    }
}
