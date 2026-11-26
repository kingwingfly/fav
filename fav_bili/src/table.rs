use std::fmt;
use std::io::{self, Write as _};
use std::str::FromStr;

use anyhow::Result;
use clap::{ValueEnum, builder::PossibleValue};
use console::{Term, strip_ansi_codes, style};
use serde::{Deserialize, Serialize};
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

/// Measure the visible terminal display width of `s`, ignoring ANSI escape codes.
pub fn display_width(s: &str) -> usize {
    strip_ansi_codes(s).width()
}

/// Write `text` followed by a newline to `stdout`.
///
/// Returns `Ok(true)` when written normally, `Ok(false)` when downstream closes the pipe
/// (`BrokenPipe`, e.g. `fav ls v | head`), and `Err` on any other I/O failure.
pub fn write_stdout_line(text: &str) -> io::Result<bool> {
    let mut out = io::stdout().lock();
    match writeln!(out, "{text}") {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(err) => Err(err),
    }
}

/// Write raw `text` (without appending an extra newline) to `stdout`, treating `BrokenPipe`
/// as a clean early exit (`Ok(false)`).
pub fn write_stdout_all(text: &str) -> io::Result<bool> {
    let mut out = io::stdout().lock();
    match out.write_all(text.as_bytes()) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(err) => Err(err),
    }
}

/// Truncate `s` to fit within `max_cols` display columns, appending `…` when truncated.
/// Never exceeds `max_cols` display columns and does not right-pad.
pub fn truncate_display(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if s.width() <= max_cols {
        return s.to_owned();
    }

    const ELLIPSIS: char = '…';
    let ellipsis_w = ELLIPSIS.width().unwrap_or(1);
    if max_cols < ellipsis_w {
        return ".".repeat(max_cols);
    }

    let budget = max_cols - ellipsis_w;
    let mut out = String::new();
    let mut cur = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
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
    let width = out.width();
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
            let cols = usize::from(term.size().1);
            (cols > 0).then_some(cols)
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
    ///
    /// When rendering to a TTY with a known terminal width, full cell text is preserved here
    /// so [`MarkdownTable::fit_to_width`] can budget column widths from the exact measured
    /// widths of all columns instead of relying on an estimated `fixed_overhead`.
    pub fn format_cell(&self, text: &str, fixed_overhead: usize) -> String {
        if self.wide || (self.is_tty && self.term_width.is_some()) {
            return text.to_owned();
        }
        match self.flexible_col_width(fixed_overhead) {
            Some(max_cols) => truncate_display(text, max_cols),
            None => text.to_owned(),
        }
    }
}

fn sanitize_table_cell(cell: &str) -> String {
    if !cell.contains(['\r', '\n']) {
        return cell.to_owned();
    }
    cell.chars()
        .map(|c| match c {
            '\r' | '\n' => ' ',
            other => other,
        })
        .collect()
}

/// Truncate `s` (which may contain ANSI CSI escape sequences) to fit within `max_cols`
/// visible display columns, appending `\x1b[0m` (if styled) and `…` when truncated.
fn truncate_ansi_display(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if display_width(s) <= max_cols {
        return s.to_owned();
    }
    if !s.contains('\x1b') {
        return truncate_display(s, max_cols);
    }

    const ELLIPSIS: char = '…';
    let ellipsis_w = ELLIPSIS.width().unwrap_or(1);
    if max_cols < ellipsis_w {
        return ".".repeat(max_cols);
    }

    let budget = max_cols - ellipsis_w;
    let mut out = String::new();
    let mut cur = 0usize;
    let mut saw_ansi = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            saw_ansi = true;
            out.push(c);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap());
                for seq_c in chars.by_ref() {
                    out.push(seq_c);
                    if ('\x40'..='\x7e').contains(&seq_c) {
                        break;
                    }
                }
            }
            continue;
        }
        let cw = c.width().unwrap_or(0);
        if cur + cw > budget {
            break;
        }
        out.push(c);
        cur += cw;
    }

    if saw_ansi {
        out.push_str("\x1b[0m");
    }
    out.push(ELLIPSIS);
    out
}

/// ANSI-aware, Unicode-width-accurate GitHub-Flavored Markdown table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownTable {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl MarkdownTable {
    /// Total rows including the header row.
    pub fn count_rows(&self) -> usize {
        self.rows.len() + 1
    }

    fn column_widths(&self) -> Vec<usize> {
        let col_count = self
            .rows
            .iter()
            .map(Vec::len)
            .fold(self.header.len(), usize::max);
        let mut col_widths = vec![0usize; col_count];
        for (idx, cell) in self.header.iter().enumerate() {
            col_widths[idx] = col_widths[idx].max(display_width(cell));
        }
        for row in &self.rows {
            for (idx, cell) in row.iter().enumerate() {
                col_widths[idx] = col_widths[idx].max(display_width(cell));
            }
        }
        col_widths
    }

    /// Shrink the widest column(s) so the rendered table fits within `max_width` terminal columns.
    pub fn fit_to_width(&mut self, max_width: usize) {
        let mut col_widths = self.column_widths();
        let col_count = col_widths.len();
        if col_count == 0 {
            return;
        }
        let borders = 3 * col_count + 1;
        let mut total = col_widths.iter().sum::<usize>() + borders;
        if total <= max_width {
            return;
        }

        // First shrink columns down to a readable floor of 4 cols, then down to 1 if still over.
        for floor in [4usize, 1usize] {
            while total > max_width {
                let Some((widest_idx, &widest_w)) = col_widths
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| **w > floor)
                    .max_by_key(|(_, w)| **w)
                else {
                    break;
                };
                let excess = total - max_width;
                let shrink = (widest_w - floor).min(excess);
                // Shrink down to at most the next-widest column (or floor) per step so
                // multiple wide columns share reductions evenly.
                let next_widest = col_widths
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != widest_idx)
                    .map(|(_, &w)| w)
                    .max()
                    .unwrap_or(floor)
                    .max(floor);
                let step = if widest_w > next_widest {
                    (widest_w - next_widest).min(shrink)
                } else {
                    1.min(shrink)
                };
                col_widths[widest_idx] -= step;
                total -= step;
            }
        }

        for (idx, cell) in self.header.iter_mut().enumerate() {
            if let Some(&limit) = col_widths.get(idx)
                && display_width(cell) > limit
            {
                *cell = truncate_ansi_display(cell, limit);
            }
        }
        for row in &mut self.rows {
            for (idx, cell) in row.iter_mut().enumerate() {
                if let Some(&limit) = col_widths.get(idx)
                    && display_width(cell) > limit
                {
                    *cell = truncate_ansi_display(cell, limit);
                }
            }
        }
    }
}

impl fmt::Display for MarkdownTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let col_widths = self.column_widths();
        if col_widths.is_empty() {
            return Ok(());
        }

        let write_row = |f: &mut fmt::Formatter<'_>, cells: &[String]| -> fmt::Result {
            f.write_str("|")?;
            for (idx, &col_w) in col_widths.iter().enumerate() {
                let cell = cells.get(idx).map(String::as_str).unwrap_or("");
                let vis_w = display_width(cell);
                let pad = col_w.saturating_sub(vis_w);
                write!(f, " {cell}{} |", " ".repeat(pad))?;
            }
            Ok(())
        };

        write_row(f, &self.header)?;
        f.write_str("\n|")?;
        for &col_w in &col_widths {
            write!(f, "{}|", "-".repeat(col_w + 2))?;
        }
        for row in &self.rows {
            f.write_str("\n")?;
            write_row(f, row)?;
        }
        Ok(())
    }
}

/// Build an ANSI-aware markdown [`MarkdownTable`] from header and row iterables.
pub fn table<H, RS, R>(header: H, records: RS) -> MarkdownTable
where
    H: IntoIterator,
    H::Item: Into<String>,
    RS: IntoIterator<Item = R>,
    R: IntoIterator,
    R::Item: Into<String>,
{
    let header = header
        .into_iter()
        .map(|cell| sanitize_table_cell(&cell.into()))
        .collect();
    let rows = records
        .into_iter()
        .map(|record| {
            record
                .into_iter()
                .map(|cell| sanitize_table_cell(&cell.into()))
                .collect()
        })
        .collect();
    MarkdownTable { header, rows }
}

/// Render a formatted table string. Fits the table to `opts.term_width` unless `opts.wide`
/// is set, and appends the `rows: N` footer only when outputting to a TTY.
pub fn render_table<H, RS, R>(header: H, records: RS, opts: TableOptions) -> String
where
    H: IntoIterator,
    H::Item: Into<String>,
    RS: IntoIterator<Item = R>,
    R: IntoIterator,
    R::Item: Into<String>,
{
    let mut tbl = table(header, records);
    if !opts.wide
        && let Some(max_width) = opts.term_width
    {
        tbl.fit_to_width(max_width);
    }
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
        assert_eq!(padded.width(), 20);
        assert_eq!(padded, format!("我的收藏{}", " ".repeat(12)));

        // Exactly 10 CJK chars = 20 display columns -> 0 trailing spaces!
        let exact = head("十个汉字的收藏夹标题", 20);
        assert_eq!(exact.width(), 20);
        assert_eq!(exact, "十个汉字的收藏夹标题");
    }

    #[test]
    fn head_and_truncate_display_mixed_ascii_and_cjk() {
        // "B站Rust教程视频" -> B(1) + 站(2) + Rust(4) + 教程视频(8) = 15 cols
        let s = "B站Rust教程视频";
        assert_eq!(s.width(), 15);
        assert_eq!(truncate_display(s, 15), s);
        assert_eq!(head(s, 15), s);

        // Truncate to 10 cols: '…' is width 1, budget = 9
        // B(1) + 站(2) + Rust(4) + 教(2) = 9, + '…'(1) = 10
        let trunc = truncate_display(s, 10);
        assert!(trunc.ends_with('…'));
        assert_eq!(trunc.width(), 10);
        assert_eq!(trunc, "B站Rust教…");

        // Truncate to 9 cols: budget = 8 -> B(1)+站(2)+Rust(4)=7, next '教' (+2=9 > 8) doesn't fit
        let trunc9 = truncate_display(s, 9);
        assert_eq!(trunc9, "B站Rust…");
        assert_eq!(trunc9.width(), 8);

        // head pads the leftover 1 display column so total width is exactly 9
        let padded = head(s, 9);
        assert_eq!(padded.width(), 9);
        assert_eq!(padded, "B站Rust… ");
    }

    #[test]
    fn truncate_display_edge_cases() {
        assert_eq!(truncate_display("hello", 0), "");
        assert_eq!(head("hello", 0), "");

        let tiny = truncate_display("中文测试", 1);
        assert_eq!(tiny.width(), 1);
        assert_eq!(head("中文测试", 1).width(), 1);

        let ascii = truncate_display("abcdefghij", 6);
        assert!(ascii.ends_with('…'));
        assert!(ascii.width() <= 6);
        assert_eq!(head("abcdefghij", 6).width(), 6);
    }

    #[test]
    fn ansi_colored_table_aligns_every_column_exact() {
        // Reproduce both `fav ls s` / `fav ls v` and `fav ls` multi-badge breakdown rows
        let sets_tbl = table(
            ["set_id", "name", "count", "state"],
            [
                [
                    "57010888".to_owned(),
                    "默认收藏夹".to_owned(),
                    "612".to_owned(),
                    format_state_badge("Active", true),
                ],
                [
                    "158322788".to_owned(),
                    "favorite".to_owned(),
                    "57".to_owned(),
                    format_state_badge("Inactive", true),
                ],
            ],
        )
        .to_string();

        let status_tbl = table(
            ["resource", "total", "breakdown"],
            [
                [
                    "Accounts".to_owned(),
                    "1".to_owned(),
                    format!(
                        "{}: 1, {}: 0, {}: 0",
                        format_state_badge("Active", true),
                        format_state_badge("Inactive", true),
                        format_state_badge("Expired", true),
                    ),
                ],
                [
                    "Medias".to_owned(),
                    "621".to_owned(),
                    format!(
                        "{}: 0, {}: 616, {}: 0, {}: 1, {}: 4, {}: 0",
                        format_state_badge("Pending", true),
                        format_state_badge("Completed", true),
                        format_state_badge("Failed", true),
                        format_state_badge("Expired", true),
                        format_state_badge("PermissionDenied", true),
                        format_state_badge("Skipped", true),
                    ),
                ],
            ],
        )
        .to_string();

        for rendered in [&sets_tbl, &status_tbl] {
            let plain_lines: Vec<String> = rendered
                .lines()
                .map(|l| strip_ansi_codes(l).into_owned())
                .collect();
            let expected_width = plain_lines[0].width();
            let pipe_positions = |line: &str| -> Vec<usize> {
                let mut pos = 0;
                let mut cols = Vec::new();
                for ch in line.chars() {
                    if ch == '|' {
                        cols.push(pos);
                    }
                    pos += ch.width().unwrap_or(0);
                }
                cols
            };
            let expected_pipes = pipe_positions(&plain_lines[0]);
            for line in &plain_lines {
                assert_eq!(
                    line.width(),
                    expected_width,
                    "misaligned line width in:\n{}",
                    plain_lines.join("\n")
                );
                assert_eq!(
                    pipe_positions(line),
                    expected_pipes,
                    "misaligned column separators in:\n{}",
                    plain_lines.join("\n")
                );
            }
        }
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

    #[test]
    fn render_table_never_exceeds_terminal_width_even_with_long_ids_and_ansi() {
        for term_cols in [80usize, 100, 120, 160] {
            let opts = TableOptions {
                wide: false,
                is_tty: true,
                colored: true,
                term_width: Some(term_cols),
            };
            let long_cjk = "这是一个非常非常长的Bilibili收藏视频标题用于验证在任何终端宽度下表格行都不会超出终端列宽限制并且对齐准确无误";
            let rendered = render_table(
                ["id", "bvid", "title", "type", "state"],
                [
                    [
                        "2129461".to_owned(),
                        "BV1zs411S7sz".to_owned(),
                        opts.format_cell("洛天依，言和原创《普通DISCO》", 64),
                        "Video".to_owned(),
                        format_state_badge("Completed", true),
                    ],
                    [
                        "114896364768316".to_owned(),
                        "BV1GPgHz4ENB".to_owned(),
                        opts.format_cell(long_cjk, 64),
                        "Video".to_owned(),
                        format_state_badge("PermissionDenied", true),
                    ],
                ],
                opts,
            );

            let table_lines: Vec<&str> = rendered.lines().filter(|l| l.starts_with('|')).collect();
            assert_eq!(table_lines.len(), 4);
            let first_w = display_width(table_lines[0]);
            assert!(
                first_w <= term_cols,
                "table width {first_w} exceeded terminal width {term_cols}:\n{rendered}"
            );
            for line in &table_lines {
                assert_eq!(display_width(line), first_w);
            }
        }
    }

    #[test]
    fn fit_to_width_truncates_ansi_colored_cells_with_reset_and_alignment() {
        let mut tbl = table(
            ["resource", "total", "breakdown"],
            [[
                "Medias".to_owned(),
                "621".to_owned(),
                format!(
                    "{}: 0, {}: 616, {}: 0, {}: 1, {}: 4, {}: 0",
                    format_state_badge("Pending", true),
                    format_state_badge("Completed", true),
                    format_state_badge("Failed", true),
                    format_state_badge("Expired", true),
                    format_state_badge("PermissionDenied", true),
                    format_state_badge("Skipped", true),
                ),
            ]],
        );
        tbl.fit_to_width(72);
        let rendered = tbl.to_string();
        for line in rendered.lines() {
            assert!(
                display_width(line) <= 72,
                "line width {} > 72: {line:?}",
                display_width(line)
            );
        }
        // Truncated styled cell must reset ANSI before `…` so colors don't bleed.
        assert!(rendered.contains("\x1b[0m…"));
    }
}
