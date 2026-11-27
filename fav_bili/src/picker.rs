use std::collections::HashSet;
use std::fmt;

use anyhow::{Result, bail};
use console::{Key, Term, style};

use crate::table::truncate_display;

/// One selectable entry shown in the interactive picker or matched on the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub id: i64,
    pub label: String,
    pub meta: String,
    pub selected: bool,
}

/// Typed error returned by [`pick_multi`] so callers can map non-interactive failures
/// to `ExitKind::Usage` cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerError {
    /// Stdin/stdout/stderr is not a TTY or `--non-interactive` was specified.
    NonInteractive { hint: String },
    /// User aborted with `Esc` or `Ctrl+C`.
    Cancelled,
    /// No items were available to choose from.
    Empty,
    /// Underlying terminal I/O failure.
    Io(String),
}

impl fmt::Display for PickerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonInteractive { hint } => {
                write!(f, "interactive picker requires a TTY; {hint}")
            }
            Self::Cancelled => f.write_str("selection cancelled"),
            Self::Empty => f.write_str("no items available to select"),
            Self::Io(msg) => write!(f, "terminal I/O error: {msg}"),
        }
    }
}

impl std::error::Error for PickerError {}

/// Score how well `query` matches `candidate` (case-insensitive).
/// Higher scores indicate better matches; `None` means no match.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let c = candidate.to_lowercase();
    if c == q {
        return Some(10_000);
    }
    if c.starts_with(&q) {
        let len_diff = c.chars().count().saturating_sub(q.chars().count()) as i64;
        return Some(5_000 - len_diff);
    }
    if let Some(byte_pos) = c.find(&q) {
        let char_pos = c[..byte_pos].chars().count() as i64;
        let len_diff = c.chars().count().saturating_sub(q.chars().count()) as i64;
        let boundary_bonus = if char_pos == 0
            || c[..byte_pos]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_alphanumeric())
        {
            200
        } else {
            0
        };
        return Some(2_000 + boundary_bonus - char_pos * 10 - len_diff);
    }

    // Subsequence fuzzy match
    let q_chars: Vec<char> = q.chars().collect();
    let c_chars: Vec<char> = c.chars().collect();
    let mut qi = 0usize;
    let mut score: i64 = 0;
    let mut prev_match: Option<usize> = None;

    for (ci, &ch) in c_chars.iter().enumerate() {
        if qi < q_chars.len() && ch == q_chars[qi] {
            let mut bonus = 10i64;
            if let Some(prev) = prev_match {
                if ci == prev + 1 {
                    bonus += 25;
                } else {
                    bonus -= (ci - prev - 1) as i64;
                }
            } else if ci == 0 {
                bonus += 30;
            }
            if ci > 0 && !c_chars[ci - 1].is_alphanumeric() {
                bonus += 15;
            }
            score += bonus;
            prev_match = Some(ci);
            qi += 1;
        }
    }

    if qi == q_chars.len() {
        let len_penalty = (c_chars.len().saturating_sub(q_chars.len())) as i64;
        Some(100 + score - len_penalty)
    } else {
        None
    }
}

/// Resolve CLI target arguments that may be either numeric IDs or case-insensitive
/// name substrings / fuzzy queries against `items`.
pub fn resolve_targets_or_fuzzy_match(
    queries: &[String],
    items: &[PickerItem],
) -> Result<Vec<i64>> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();

    for raw in queries {
        let q = raw.trim();
        if q.is_empty() {
            bail!("empty target query");
        }

        let matched_id = if let Ok(id) = q.parse::<i64>() {
            if items.iter().any(|it| it.id == id) {
                id
            } else {
                bail!("target ID {id} not found");
            }
        } else {
            let q_lower = q.to_lowercase();

            // 1. Exact case-insensitive label match
            let exact: Vec<&PickerItem> = items
                .iter()
                .filter(|it| it.label.to_lowercase() == q_lower)
                .collect();
            if exact.len() == 1 {
                exact[0].id
            } else if exact.len() > 1 {
                bail!("{}", format_ambiguous(q, &exact));
            } else {
                // 2. Case-insensitive substring match
                let substr: Vec<&PickerItem> = items
                    .iter()
                    .filter(|it| it.label.to_lowercase().contains(&q_lower))
                    .collect();
                if substr.len() == 1 {
                    substr[0].id
                } else if substr.len() > 1 {
                    bail!("{}", format_ambiguous(q, &substr));
                } else {
                    // 3. Fuzzy subsequence match
                    let mut scored: Vec<(i64, &PickerItem)> = items
                        .iter()
                        .filter_map(|it| fuzzy_score(q, &it.label).map(|s| (s, it)))
                        .collect();
                    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
                    match scored.len() {
                        0 => bail!("no item matching '{q}'"),
                        1 => scored[0].1.id,
                        _ => {
                            let candidates: Vec<&PickerItem> =
                                scored.into_iter().take(5).map(|(_, it)| it).collect();
                            bail!("{}", format_ambiguous(q, &candidates));
                        }
                    }
                }
            }
        };

        if seen.insert(matched_id) {
            resolved.push(matched_id);
        }
    }

    Ok(resolved)
}

fn format_ambiguous(query: &str, matches: &[&PickerItem]) -> String {
    let preview: Vec<String> = matches
        .iter()
        .take(5)
        .map(|it| format!("{} ({})", it.id, it.label))
        .collect();
    format!(
        "ambiguous target '{query}' matched {} items: {}; specify a numeric ID or a more specific name",
        matches.len(),
        preview.join(", ")
    )
}

#[derive(Debug, PartialEq, Eq)]
enum PickerAction {
    Continue,
    Confirm,
    Cancel,
}

#[derive(Debug)]
struct PickerState {
    items: Vec<PickerItem>,
    selected: Vec<bool>,
    filter: String,
    cursor: usize,
    scroll: usize,
}

impl PickerState {
    fn new(items: Vec<PickerItem>) -> Self {
        let selected = items.iter().map(|i| i.selected).collect();
        Self {
            items,
            selected,
            filter: String::new(),
            cursor: 0,
            scroll: 0,
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.trim().is_empty() {
            return (0..self.items.len()).collect();
        }
        let mut scored: Vec<(i64, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                let id_str = item.id.to_string();
                let best = [
                    fuzzy_score(&self.filter, &item.label),
                    fuzzy_score(&self.filter, &id_str),
                    fuzzy_score(&self.filter, &item.meta),
                ]
                .into_iter()
                .flatten()
                .max();
                best.map(|s| (s, idx))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, idx)| idx).collect()
    }

    fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    fn selected_ids(&self) -> Vec<i64> {
        self.items
            .iter()
            .zip(self.selected.iter())
            .filter_map(|(item, &sel)| sel.then_some(item.id))
            .collect()
    }

    fn clamp_cursor(&mut self, visible_rows: usize) {
        let count = self.filtered_indices().len();
        if count == 0 {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        if self.cursor >= count {
            self.cursor = count - 1;
        }
        let page = visible_rows.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + page {
            self.scroll = self.cursor + 1 - page;
        }
    }

    fn handle_key(&mut self, key: Key, visible_rows: usize) -> PickerAction {
        let filtered = self.filtered_indices();
        match key {
            Key::Enter => {
                if self.selected_count() == 0
                    && let Some(&item_idx) = filtered.get(self.cursor)
                {
                    self.selected[item_idx] = true;
                }
                return PickerAction::Confirm;
            }
            Key::Escape => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.cursor = 0;
                    self.scroll = 0;
                } else {
                    return PickerAction::Cancel;
                }
            }
            Key::Char('\x03') => return PickerAction::Cancel,
            Key::ArrowUp | Key::Char('\x10') => {
                if !filtered.is_empty() {
                    if self.cursor == 0 {
                        self.cursor = filtered.len() - 1;
                    } else {
                        self.cursor -= 1;
                    }
                }
            }
            Key::ArrowDown | Key::Char('\x0e') => {
                if !filtered.is_empty() {
                    self.cursor = (self.cursor + 1) % filtered.len();
                }
            }
            Key::Tab | Key::Char('\x00') => {
                if let Some(&item_idx) = filtered.get(self.cursor) {
                    self.selected[item_idx] = !self.selected[item_idx];
                }
            }
            Key::Char(' ') if self.filter.is_empty() => {
                if let Some(&item_idx) = filtered.get(self.cursor) {
                    self.selected[item_idx] = !self.selected[item_idx];
                }
            }
            Key::Char('\x01') => {
                // Ctrl+A: toggle all currently filtered items
                if !filtered.is_empty() {
                    let all_selected = filtered.iter().all(|&idx| self.selected[idx]);
                    for idx in filtered {
                        self.selected[idx] = !all_selected;
                    }
                }
            }
            Key::Backspace => {
                self.filter.pop();
                self.cursor = 0;
                self.scroll = 0;
            }
            Key::Char(c) if !c.is_control() => {
                self.filter.push(c);
                self.cursor = 0;
                self.scroll = 0;
            }
            _ => {}
        }
        self.clamp_cursor(visible_rows);
        PickerAction::Continue
    }
}

struct TerminalGuard<'a> {
    term: &'a Term,
    rendered_lines: usize,
}

impl<'a> TerminalGuard<'a> {
    fn new(term: &'a Term) -> std::result::Result<Self, PickerError> {
        term.hide_cursor()
            .map_err(|e| PickerError::Io(e.to_string()))?;
        Ok(Self {
            term,
            rendered_lines: 0,
        })
    }

    fn clear_rendered(&mut self) -> std::result::Result<(), PickerError> {
        if self.rendered_lines > 0 {
            self.term
                .clear_last_lines(self.rendered_lines)
                .map_err(|e| PickerError::Io(e.to_string()))?;
            self.rendered_lines = 0;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard<'_> {
    fn drop(&mut self) {
        if self.rendered_lines > 0 {
            let _ = self.term.clear_last_lines(self.rendered_lines);
        }
        let _ = self.term.show_cursor();
        let _ = self.term.flush();
    }
}

/// Launch an interactive TTY-only fuzzy multi-select picker rendered on `stderr`.
/// Immediately returns [`PickerError::NonInteractive`] if `non_interactive` is set
/// or either `stdout`/`stderr` is not attached to a terminal.
pub fn pick_multi(
    title: &str,
    items: &[PickerItem],
    non_interactive: bool,
    usage_hint: &str,
) -> std::result::Result<Vec<i64>, PickerError> {
    let stderr = Term::stderr();
    let stdout = Term::stdout();
    if non_interactive || !stderr.is_term() || !stdout.is_term() {
        return Err(PickerError::NonInteractive {
            hint: usage_hint.to_owned(),
        });
    }
    if items.is_empty() {
        return Err(PickerError::Empty);
    }

    let mut state = PickerState::new(items.to_vec());
    let mut guard = TerminalGuard::new(&stderr)?;

    loop {
        let (rows, cols) = stderr.size();
        let visible_rows = (usize::from(rows).saturating_sub(5)).clamp(5, 15);
        let term_cols = usize::from(cols).max(40);
        state.clamp_cursor(visible_rows);

        guard.clear_rendered()?;
        let lines = render_frame(title, &state, visible_rows, term_cols);
        for line in &lines {
            stderr
                .write_line(line)
                .map_err(|e| PickerError::Io(e.to_string()))?;
        }
        stderr.flush().map_err(|e| PickerError::Io(e.to_string()))?;
        guard.rendered_lines = lines.len();

        let key = match stderr.read_key() {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                return Err(PickerError::Cancelled);
            }
            Err(e) => return Err(PickerError::Io(e.to_string())),
        };

        match state.handle_key(key, visible_rows) {
            PickerAction::Continue => {}
            PickerAction::Confirm => {
                guard.clear_rendered()?;
                return Ok(state.selected_ids());
            }
            PickerAction::Cancel => {
                guard.clear_rendered()?;
                return Err(PickerError::Cancelled);
            }
        }
    }
}

fn render_frame(
    title: &str,
    state: &PickerState,
    visible_rows: usize,
    term_cols: usize,
) -> Vec<String> {
    let filtered = state.filtered_indices();
    let mut lines = Vec::new();

    let header = format!(
        "{} ({}/{} selected)  [↑↓ move, Tab toggle, Ctrl-A all, Enter confirm, Esc clear/cancel]",
        title,
        state.selected_count(),
        state.items.len()
    );
    lines.push(
        style(truncate_display(&header, term_cols))
            .bold()
            .cyan()
            .to_string(),
    );

    let filter_line = format!(
        "Filter: {}_  ({}/{} matches)",
        state.filter,
        filtered.len(),
        state.items.len()
    );
    lines.push(truncate_display(&filter_line, term_cols));

    if filtered.is_empty() {
        lines.push(style("  (no matching items)").dim().to_string());
    } else {
        let end = (state.scroll + visible_rows).min(filtered.len());
        for (view_idx, &item_idx) in filtered[state.scroll..end].iter().enumerate() {
            let abs_view_idx = state.scroll + view_idx;
            let item = &state.items[item_idx];
            let is_cur = abs_view_idx == state.cursor;
            let is_sel = state.selected[item_idx];

            let pointer = if is_cur { ">" } else { " " };
            let check = if is_sel { "x" } else { " " };
            let meta_suffix = if item.meta.is_empty() {
                String::new()
            } else {
                format!("  [{}]", item.meta)
            };
            let raw = format!(
                "{pointer} [{check}] {}  {}{meta_suffix}",
                item.id, item.label
            );
            let truncated = truncate_display(&raw, term_cols);
            if is_cur {
                lines.push(style(truncated).bold().to_string());
            } else if is_sel {
                lines.push(style(truncated).green().to_string());
            } else {
                lines.push(truncated);
            }
        }
        if filtered.len() > visible_rows {
            lines.push(
                style(format!(
                    "  showing {}-{} of {}",
                    state.scroll + 1,
                    end,
                    filtered.len()
                ))
                .dim()
                .to_string(),
            );
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_items() -> Vec<PickerItem> {
        vec![
            PickerItem {
                id: 1001,
                label: "Rust Tutorials".into(),
                meta: "42 videos, Inactive".into(),
                selected: false,
            },
            PickerItem {
                id: 1002,
                label: "Music Live".into(),
                meta: "18 videos, Active".into(),
                selected: true,
            },
            PickerItem {
                id: 1003,
                label: "默认收藏夹".into(),
                meta: "120 videos, Inactive".into(),
                selected: false,
            },
            PickerItem {
                id: 1004,
                label: "Rust Async Deep Dive".into(),
                meta: "10 videos, Inactive".into(),
                selected: false,
            },
        ]
    }

    #[test]
    fn fuzzy_score_ranks_exact_prefix_substring_subsequence() {
        let exact = fuzzy_score("rust", "Rust").unwrap();
        let prefix = fuzzy_score("rust", "Rust Tutorials").unwrap();
        let substr = fuzzy_score("rust", "Learn Rust Fast").unwrap();
        let subseq = fuzzy_score("rst", "Rust Tutorials").unwrap();

        assert!(exact > prefix);
        assert!(prefix > substr);
        assert!(substr > subseq);
        assert!(fuzzy_score("python", "Rust Tutorials").is_none());
        assert!(fuzzy_score("收藏", "默认收藏夹").is_some());
    }

    #[test]
    fn resolve_targets_numeric_exact_substring_fuzzy_and_ambiguous() {
        let items = sample_items();

        // Numeric ID + deduplication
        let ids = resolve_targets_or_fuzzy_match(&["1002".into(), "1002".into()], &items).unwrap();
        assert_eq!(ids, vec![1002]);

        // Missing numeric ID errors cleanly
        let err = resolve_targets_or_fuzzy_match(&["9999".into()], &items).unwrap_err();
        assert!(err.to_string().contains("9999"));

        // Exact case-insensitive match
        let exact = resolve_targets_or_fuzzy_match(&["music live".into()], &items).unwrap();
        assert_eq!(exact, vec![1002]);

        // Unique CJK substring match
        let cjk = resolve_targets_or_fuzzy_match(&["收藏夹".into()], &items).unwrap();
        assert_eq!(cjk, vec![1003]);

        // Unique fuzzy subsequence match
        let fuzzy = resolve_targets_or_fuzzy_match(&["msclv".into()], &items).unwrap();
        assert_eq!(fuzzy, vec![1002]);

        // Ambiguous substring ("rust" matches 1001 and 1004)
        let ambig = resolve_targets_or_fuzzy_match(&["rust".into()], &items).unwrap_err();
        let msg = ambig.to_string();
        assert!(msg.contains("ambiguous"));
        assert!(msg.contains("1001"));
        assert!(msg.contains("1004"));

        // No match
        assert!(resolve_targets_or_fuzzy_match(&["nonexistent".into()], &items).is_err());
    }

    #[test]
    fn pick_multi_rejects_non_interactive() {
        let items = sample_items();
        let err = pick_multi(
            "Select sets",
            &items,
            true,
            "pass explicit set IDs or --all",
        )
        .unwrap_err();
        assert_eq!(
            err,
            PickerError::NonInteractive {
                hint: "pass explicit set IDs or --all".into()
            }
        );
    }

    #[test]
    fn picker_state_navigation_filter_and_toggle() {
        let mut state = PickerState::new(sample_items());
        assert_eq!(state.selected_ids(), vec![1002]);

        // Space toggles when filter is empty (item 0 = 1001)
        assert_eq!(state.handle_key(Key::Char(' '), 10), PickerAction::Continue);
        assert_eq!(state.selected_ids(), vec![1001, 1002]);

        // Type filter "rust" -> matches 1001 and 1004
        for ch in "rust".chars() {
            state.handle_key(Key::Char(ch), 10);
        }
        assert_eq!(state.filtered_indices(), vec![0, 3]);

        // Space while filter is non-empty appends to filter
        state.handle_key(Key::Char(' '), 10);
        assert_eq!(state.filter, "rust ");
        state.handle_key(Key::Backspace, 10);
        assert_eq!(state.filter, "rust");

        // Ctrl+A selects all filtered (1001 and 1004)
        state.handle_key(Key::Char('\x01'), 10);
        assert_eq!(state.selected_ids(), vec![1001, 1002, 1004]);

        // Ctrl+A again deselects all filtered (leaves 1002 untouched!)
        state.handle_key(Key::Char('\x01'), 10);
        assert_eq!(state.selected_ids(), vec![1002]);

        // Move down to 1004 and toggle with Tab
        state.handle_key(Key::ArrowDown, 10);
        state.handle_key(Key::Tab, 10);
        assert_eq!(state.selected_ids(), vec![1002, 1004]);

        assert_eq!(state.handle_key(Key::Enter, 10), PickerAction::Confirm);
        // First Escape clears non-empty filter without losing selections
        assert_eq!(state.handle_key(Key::Escape, 10), PickerAction::Continue);
        assert!(state.filter.is_empty());
        assert_eq!(state.selected_ids(), vec![1002, 1004]);
        // Second Escape (with empty filter) cancels
        assert_eq!(state.handle_key(Key::Escape, 10), PickerAction::Cancel);
    }

    #[test]
    fn picker_enter_fallback_selects_highlighted_only_when_nothing_toggled() {
        let mut items = sample_items();
        for item in &mut items {
            item.selected = false;
        }

        // 0 toggled, cursor moved to item index 2 (1003) -> Enter selects 1003
        let mut state = PickerState::new(items.clone());
        state.handle_key(Key::ArrowDown, 10);
        state.handle_key(Key::ArrowDown, 10);
        assert_eq!(state.selected_count(), 0);
        assert_eq!(state.handle_key(Key::Enter, 10), PickerAction::Confirm);
        assert_eq!(state.selected_ids(), vec![1003]);

        // 0 toggled, filtered to "rust" ([1001, 1004]), move down to 1004 -> Enter selects 1004
        let mut state = PickerState::new(items.clone());
        for ch in "rust".chars() {
            state.handle_key(Key::Char(ch), 10);
        }
        state.handle_key(Key::ArrowDown, 10);
        assert_eq!(state.handle_key(Key::Enter, 10), PickerAction::Confirm);
        assert_eq!(state.selected_ids(), vec![1004]);

        // >=1 toggled -> Enter returns ONLY toggled items, NOT cursor item
        let mut state = PickerState::new(items.clone());
        state.handle_key(Key::Tab, 10); // toggles 1001
        state.handle_key(Key::ArrowDown, 10); // cursor on 1002 (untoggled)
        assert_eq!(state.handle_key(Key::Enter, 10), PickerAction::Confirm);
        assert_eq!(state.selected_ids(), vec![1001]);

        // 0 toggled, empty filtered list -> Enter returns empty
        let mut state = PickerState::new(items);
        for ch in "no-such-match-xyz".chars() {
            state.handle_key(Key::Char(ch), 10);
        }
        assert!(state.filtered_indices().is_empty());
        assert_eq!(state.handle_key(Key::Enter, 10), PickerAction::Confirm);
        assert!(state.selected_ids().is_empty());
    }

    #[test]
    fn picker_escape_clears_filter_first_and_ctrl_c_cancels_immediately() {
        let mut state = PickerState::new(sample_items());
        // Select 1001 in addition to pre-selected 1002
        state.handle_key(Key::Tab, 10);
        assert_eq!(state.selected_ids(), vec![1001, 1002]);

        // Filter and move cursor/scroll
        for ch in "rust".chars() {
            state.handle_key(Key::Char(ch), 1);
        }
        state.handle_key(Key::ArrowDown, 1);
        assert_eq!(state.cursor, 1);
        assert_eq!(state.scroll, 1);

        // First Esc clears filter, resets cursor & scroll, preserves selections
        assert_eq!(state.handle_key(Key::Escape, 1), PickerAction::Continue);
        assert!(state.filter.is_empty());
        assert_eq!(state.cursor, 0);
        assert_eq!(state.scroll, 0);
        assert_eq!(state.selected_ids(), vec![1001, 1002]);

        // Second Esc with empty filter cancels
        assert_eq!(state.handle_key(Key::Escape, 1), PickerAction::Cancel);

        // Ctrl-C cancels immediately even when filter is non-empty
        state.handle_key(Key::Char('r'), 10);
        assert!(!state.filter.is_empty());
        assert_eq!(
            state.handle_key(Key::Char('\x03'), 10),
            PickerAction::Cancel
        );
    }

    #[test]
    fn picker_render_frame_header_reflects_honest_controls() {
        let state = PickerState::new(sample_items());
        let lines = render_frame("Activate sets", &state, 10, 120);
        let plain_header = console::strip_ansi_codes(&lines[0]);
        assert!(
            plain_header
                .contains("[↑↓ move, Tab toggle, Ctrl-A all, Enter confirm, Esc clear/cancel]"),
            "unexpected header: {plain_header}"
        );
        assert!(!plain_header.contains("Tab/Space"));
    }
}
