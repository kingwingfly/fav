use std::fmt;
use std::str::FromStr;

use anyhow::Result;
use clap::{ValueEnum, builder::PossibleValue};
use console::{Term, style};
use serde::{Deserialize, Serialize};
use tabled::{Table, builder::Builder, settings::Style};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Jsonl,
    Tsv,
    Ids,
}

impl ValueEnum for OutputFormat {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Table, Self::Json, Self::Jsonl, Self::Tsv, Self::Ids]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            Self::Table => PossibleValue::new("table"),
            Self::Json => PossibleValue::new("json"),
            Self::Jsonl => PossibleValue::new("jsonl"),
            Self::Tsv => PossibleValue::new("tsv"),
            Self::Ids => PossibleValue::new("ids"),
        })
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Table => "table",
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::Tsv => "tsv",
            Self::Ids => "ids",
        })
    }
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "jsonl" => Ok(Self::Jsonl),
            "tsv" => Ok(Self::Tsv),
            "ids" => Ok(Self::Ids),
            other => Err(format!(
                "invalid output format '{other}' (expected one of: table, json, jsonl, tsv, ids)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ValueEnum for ColorChoice {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Auto, Self::Always, Self::Never]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            Self::Auto => PossibleValue::new("auto"),
            Self::Always => PossibleValue::new("always"),
            Self::Never => PossibleValue::new("never"),
        })
    }
}

impl fmt::Display for ColorChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

impl FromStr for ColorChoice {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            other => Err(format!(
                "invalid color choice '{other}' (expected one of: auto, always, never)"
            )),
        }
    }
}

/// Determine whether ANSI colors should be emitted, honoring the `NO_COLOR` specification.
pub fn should_color(choice: ColorChoice, is_tty: bool) -> bool {
    let no_color = std::env::var("NO_COLOR").ok();
    should_color_with_env(choice, is_tty, no_color.as_deref())
}

/// Pure, testable variant of [`should_color`] taking an explicit `NO_COLOR` value.
pub fn should_color_with_env(
    choice: ColorChoice,
    is_tty: bool,
    no_color_env: Option<&str>,
) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            let no_color_set = no_color_env.is_some_and(|v| !v.is_empty());
            is_tty && !no_color_set
        }
    }
}

/// Truncate `s` to fit within `max_cols` CJK display columns, appending `…` when truncated.
/// Never exceeds `max_cols` display columns and does not right-pad.
pub fn truncate_display(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if s.width_cjk() <= max_cols {
        return s.to_owned();
    }

    const ELLIPSIS: char = '…';
    let ellipsis_w = ELLIPSIS.width_cjk().unwrap_or(1);
    if max_cols < ellipsis_w {
        return ".".repeat(max_cols);
    }

    let budget = max_cols - ellipsis_w;
    let mut out = String::new();
    let mut cur = 0;
    for c in s.chars() {
        let cw = c.width_cjk().unwrap_or(0);
        if cur + cw > budget {
            break;
        }
        out.push(c);
        cur += cw;
    }
    out.push(ELLIPSIS);
    out
}

/// Return `s` truncated to `max_cols` display columns (with `…` if truncated) and
/// right-padded with ASCII spaces so the returned string occupies exactly `max_cols`
/// display columns.
pub fn head(s: impl AsRef<str>, max_cols: usize) -> String {
    let mut out = truncate_display(s.as_ref(), max_cols);
    let width = out.width_cjk();
    if width < max_cols {
        out.extend(std::iter::repeat_n(' ', max_cols - width));
    }
    out
}

/// Format an entity state string with optional ANSI color styling.
pub fn format_state_badge(state: &str, colored: bool) -> String {
    if !colored {
        return state.to_owned();
    }
    match state.to_ascii_lowercase().as_str() {
        "active" | "completed" => style(state).force_styling(true).green().to_string(),
        "pending" | "downloading" => style(state).force_styling(true).yellow().to_string(),
        "failed" | "expired" | "permissiondenied" | "permission_denied" | "unreachable" => {
            style(state).force_styling(true).red().to_string()
        }
        "inactive" | "skipped" | "deactivated" => {
            style(state).force_styling(true).dim().to_string()
        }
        _ => state.to_owned(),
    }
}

/// Options governing table layout and decorations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableOptions {
    pub wide: bool,
    pub is_tty: bool,
    pub colored: bool,
    pub term_width: Option<usize>,
}

impl TableOptions {
    /// Detect terminal capabilities for `stdout`.
    pub fn detect(wide: bool, color: ColorChoice) -> Self {
        let term = Term::stdout();
        let is_tty = term.is_term();
        let term_width = if is_tty {
            Some(usize::from(term.size().1))
        } else {
            None
        };
        let colored = should_color(color, is_tty);
        Self {
            wide,
            is_tty,
            colored,
            term_width,
        }
    }

    /// Compute the target display width for a flexible text column (such as title or name).
    /// Returns `None` when `--wide` / `--no-truncate` is active.
    pub fn flexible_col_width(&self, fixed_overhead: usize) -> Option<usize> {
        if self.wide {
            return None;
        }
        let term_cols = self.term_width.unwrap_or(100);
        let available = term_cols.saturating_sub(fixed_overhead);
        Some(available.clamp(20, 120))
    }

    /// Format a flexible text column according to `wide` and terminal width.
    pub fn format_cell(&self, text: &str, fixed_overhead: usize) -> String {
        match self.flexible_col_width(fixed_overhead) {
            Some(max_cols) => truncate_display(text, max_cols),
            None => text.to_owned(),
        }
    }
}

/// Build a markdown-styled [`Table`] from header and row iterables.
pub fn table<H, RS, R>(header: H, records: RS) -> Table
where
    H: IntoIterator,
    H::Item: Into<String>,
    RS: IntoIterator<Item = R>,
    R: IntoIterator,
    R::Item: Into<String>,
{
    let mut builder = Builder::new();
    builder.push_record(header);
    for record in records {
        builder.push_record(record);
    }
    let mut table = builder.build();
    table.with(Style::markdown());
    table
}

/// Render a formatted table string. Appends the `rows: N` footer only when outputting to a TTY.
pub fn render_table<H, RS, R>(header: H, records: RS, opts: TableOptions) -> String
where
    H: IntoIterator,
    H::Item: Into<String>,
    RS: IntoIterator<Item = R>,
    R: IntoIterator,
    R::Item: Into<String>,
{
    let tbl = table(header, records);
    let row_count = tbl.count_rows().saturating_sub(1);
    if opts.is_tty {
        format!("{tbl}\nrows: {row_count}")
    } else {
        tbl.to_string()
    }
}

/// Render a slice of serializable items as pretty-printed JSON.
pub fn render_json<T: Serialize>(items: &[T]) -> Result<String> {
    Ok(serde_json::to_string_pretty(items)?)
}

/// Render a slice of serializable items as newline-delimited JSON (JSONL).
pub fn render_jsonl<T: Serialize>(items: &[T]) -> Result<String> {
    let mut lines = Vec::with_capacity(items.len());
    for item in items {
        lines.push(serde_json::to_string(item)?);
    }
    Ok(lines.join("\n"))
}

fn sanitize_tsv_cell(cell: &str) -> String {
    cell.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

/// Render header and records as tab-separated values (TSV), sanitizing embedded tabs/newlines.
pub fn render_tsv<H, RS, R>(header: H, records: RS) -> String
where
    H: IntoIterator,
    H::Item: AsRef<str>,
    RS: IntoIterator<Item = R>,
    R: IntoIterator,
    R::Item: AsRef<str>,
{
    let mut lines = Vec::new();
    let header_line: Vec<String> = header
        .into_iter()
        .map(|c| sanitize_tsv_cell(c.as_ref()))
        .collect();
    lines.push(header_line.join("\t"));
    for record in records {
        let row: Vec<String> = record
            .into_iter()
            .map(|c| sanitize_tsv_cell(c.as_ref()))
            .collect();
        lines.push(row.join("\t"));
    }
    lines.join("\n")
}

/// Render bare IDs, one per line.
pub fn render_ids<I, S>(ids: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    ids.into_iter()
        .map(|id| id.as_ref().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct SampleRow {
        id: i64,
        title: String,
        state: String,
    }

    #[test]
    fn head_pads_by_cjk_display_width_not_char_count() {
        // 4 CJK chars = 8 display columns -> should pad with 12 spaces (total 20 display cols)
        let padded = head("我的收藏", 20);
        assert_eq!(padded.width_cjk(), 20);
        assert_eq!(padded, format!("我的收藏{}", " ".repeat(12)));

        // Exactly 10 CJK chars = 20 display columns -> 0 trailing spaces!
        let exact = head("十个汉字的收藏夹标题", 20);
        assert_eq!(exact.width_cjk(), 20);
        assert_eq!(exact, "十个汉字的收藏夹标题");
    }

    #[test]
    fn head_and_truncate_display_mixed_ascii_and_cjk() {
        // "B站Rust教程视频" -> B(1) + 站(2) + Rust(4) + 教程视频(8) = 15 cols
        let s = "B站Rust教程视频";
        assert_eq!(s.width_cjk(), 15);
        assert_eq!(truncate_display(s, 15), s);
        assert_eq!(head(s, 15), s);

        // Truncate to 10 cols: '…' is width 2 under width_cjk(), budget = 8
        // B(1) + 站(2) + Rust(4) = 7, next char '教' (+2 = 9 > 8) doesn't fit
        let trunc = truncate_display(s, 10);
        assert!(trunc.ends_with('…'));
        assert!(trunc.width_cjk() <= 10);
        assert_eq!(trunc, "B站Rust…");
        assert_eq!(trunc.width_cjk(), 9);

        // head pads the leftover 1 display column so total width is exactly 10
        let padded = head(s, 10);
        assert_eq!(padded.width_cjk(), 10);
        assert_eq!(padded, "B站Rust… ");
    }

    #[test]
    fn truncate_display_edge_cases() {
        assert_eq!(truncate_display("hello", 0), "");
        assert_eq!(head("hello", 0), "");

        let tiny = truncate_display("中文测试", 1);
        assert_eq!(tiny.width_cjk(), 1);
        assert_eq!(head("中文测试", 1).width_cjk(), 1);

        let ascii = truncate_display("abcdefghij", 6);
        assert!(ascii.ends_with('…'));
        assert!(ascii.width_cjk() <= 6);
        assert_eq!(head("abcdefghij", 6).width_cjk(), 6);
    }

    #[test]
    fn should_color_respects_choice_tty_and_no_color() {
        assert!(should_color_with_env(ColorChoice::Always, false, Some("1")));
        assert!(!should_color_with_env(ColorChoice::Never, true, None));
        assert!(should_color_with_env(ColorChoice::Auto, true, None));
        assert!(should_color_with_env(ColorChoice::Auto, true, Some("")));
        assert!(!should_color_with_env(ColorChoice::Auto, true, Some("1")));
        assert!(!should_color_with_env(ColorChoice::Auto, false, None));
    }

    #[test]
    fn format_state_badge_plain_and_colored() {
        assert_eq!(format_state_badge("Active", false), "Active");
        assert_eq!(format_state_badge("Pending", false), "Pending");
        assert_eq!(format_state_badge("Failed", false), "Failed");
        assert_eq!(format_state_badge("Skipped", false), "Skipped");

        for state in [
            "Active",
            "Completed",
            "Pending",
            "Downloading",
            "Failed",
            "Expired",
            "PermissionDenied",
            "Inactive",
            "Skipped",
        ] {
            let colored = format_state_badge(state, true);
            assert!(
                colored.contains("\x1b["),
                "expected ANSI sequence for {state}, got {colored:?}"
            );
            assert!(colored.contains(state));
        }
    }

    #[test]
    fn render_table_footer_depends_on_tty() {
        let tty_opts = TableOptions {
            wide: false,
            is_tty: true,
            colored: false,
            term_width: Some(100),
        };
        let pipe_opts = TableOptions {
            is_tty: false,
            ..tty_opts
        };

        let out_tty = render_table(["id", "name"], [["1", "alpha"], ["2", "beta"]], tty_opts);
        assert!(out_tty.ends_with("\nrows: 2"));

        let out_pipe = render_table(["id", "name"], [["1", "alpha"], ["2", "beta"]], pipe_opts);
        assert!(!out_pipe.contains("rows:"));
    }

    #[test]
    fn render_json_jsonl_tsv_ids() {
        let items = vec![
            SampleRow {
                id: 101,
                title: "First\tVideo".into(),
                state: "Completed".into(),
            },
            SampleRow {
                id: 202,
                title: "Second\nVideo".into(),
                state: "Pending".into(),
            },
        ];

        let json = render_json(&items).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["id"], 101);

        let jsonl = render_jsonl(&items).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["state"], "Completed");

        let tsv = render_tsv(
            ["id", "title", "state"],
            items
                .iter()
                .map(|r| [r.id.to_string(), r.title.clone(), r.state.clone()]),
        );
        assert_eq!(
            tsv,
            "id\ttitle\tstate\n101\tFirst Video\tCompleted\n202\tSecond Video\tPending"
        );

        let ids = render_ids(items.iter().map(|r| r.id.to_string()));
        assert_eq!(ids, "101\n202");
    }

    #[test]
    fn wide_option_disables_truncation() {
        let opts = TableOptions {
            wide: true,
            is_tty: true,
            colored: false,
            term_width: Some(80),
        };
        assert_eq!(opts.flexible_col_width(40), None);
        let long_title = "这是一个非常长的Bilibili视频标题用于测试宽屏不截断功能";
        assert_eq!(opts.format_cell(long_title, 40), long_title);
    }
}
