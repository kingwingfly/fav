use std::str::FromStr as _;

use anyhow::anyhow;

use crate::{
    context::AppContext,
    db::{AccountFilter, MediaFilter, MediaSort, SetFilter, UpFilter},
    error::{AppError, AppResult},
    state::{AccountState, MediaState, SetKind, SetState, UpState},
    table::{
        OutputFormat, TableOptions, format_state_badge, render_ids, render_json, render_jsonl,
        render_table, render_tsv, write_stdout_line,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Account,
    Set,
    Up,
    Media,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub format: OutputFormat,
    pub wide: bool,
    pub state: Option<String>,
    pub kind: Option<String>,
    pub set_id: Option<i64>,
    pub up_id: Option<i64>,
    pub account_id: Option<i64>,
    pub active_only: bool,
    pub inactive_only: bool,
    pub media_type: Option<String>,
    pub search: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u64>,
}

fn parse_sort_spec(raw: Option<&str>) -> Option<(String, bool)> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('-') {
        return Some((rest.to_ascii_lowercase(), true));
    }
    if let Some((field, dir)) = s.split_once(':') {
        let desc = dir.eq_ignore_ascii_case("desc");
        return Some((field.to_ascii_lowercase(), desc));
    }
    Some((s.to_ascii_lowercase(), false))
}

fn parse_state_list<T>(raw: Option<&str>) -> AppResult<Vec<T>>
where
    T: std::str::FromStr<Err = String>,
{
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
    for token in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let state = T::from_str(token).map_err(|e| AppError::usage(anyhow!("{e}")))?;
        parsed.push(state);
    }
    Ok(parsed)
}

/// Render a list query into a formatted string (useful for both CLI printing and unit tests).
pub async fn render_list(
    ctx: &AppContext,
    kind: ListKind,
    opts: &ListOptions,
) -> AppResult<(String, usize)> {
    if opts.active_only && opts.inactive_only {
        return Err(AppError::usage(anyhow!(
            "--active and --inactive cannot be used together"
        )));
    }
    let table_opts = TableOptions::detect(opts.wide, ctx.runtime.color);

    match kind {
        ListKind::Account => {
            let mut states = parse_state_list::<AccountState>(opts.state.as_deref())?;
            if opts.active_only {
                states.push(AccountState::Active);
            } else if opts.inactive_only {
                states.push(AccountState::Inactive);
            }
            let sort = parse_sort_spec(opts.sort.as_deref());
            let mut rows = ctx
                .db
                .query_accounts(&AccountFilter {
                    states,
                    search: opts.search.clone(),
                    limit: if sort.is_none() { opts.limit } else { None },
                })
                .await?;
            if let Some((field, desc)) = sort {
                match field.as_str() {
                    "id" | "account_id" => rows.sort_by_key(|a| a.account_id),
                    "name" => rows.sort_by(|a, b| {
                        a.name
                            .to_lowercase()
                            .cmp(&b.name.to_lowercase())
                            .then_with(|| a.account_id.cmp(&b.account_id))
                    }),
                    "state" => rows.sort_by(|a, b| {
                        a.state
                            .cmp(&b.state)
                            .then_with(|| a.account_id.cmp(&b.account_id))
                    }),
                    other => {
                        return Err(AppError::usage(anyhow!(
                            "invalid sort field '{other}' for accounts (expected: id, name, state)"
                        )));
                    }
                }
                if desc {
                    rows.reverse();
                }
                if let Some(limit) = opts.limit {
                    rows.truncate(limit as usize);
                }
            }
            let count = rows.len();
            let rendered = match opts.format {
                OutputFormat::Table => render_table(
                    ["account_id", "name", "state"],
                    rows.iter().map(|a| {
                        [
                            a.account_id.to_string(),
                            table_opts.format_cell(&a.name, 30),
                            format_state_badge(&a.state, table_opts.colored),
                        ]
                    }),
                    table_opts,
                ),
                OutputFormat::Json => render_json(&rows)?,
                OutputFormat::Jsonl => render_jsonl(&rows)?,
                OutputFormat::Tsv => render_tsv(
                    ["account_id", "name", "state"],
                    rows.iter()
                        .map(|a| [a.account_id.to_string(), a.name.clone(), a.state.clone()]),
                ),
                OutputFormat::Ids => render_ids(rows.iter().map(|a| a.account_id.to_string())),
            };
            Ok((rendered, count))
        }
        ListKind::Set => {
            let mut states = parse_state_list::<SetState>(opts.state.as_deref())?;
            if opts.inactive_only {
                states.push(SetState::Inactive);
            }
            let set_kind = match opts.kind.as_deref() {
                Some(raw) if !raw.trim().is_empty() => Some(
                    SetKind::from_str(raw.trim()).map_err(|e| AppError::usage(anyhow!("{e}")))?,
                ),
                _ => None,
            };
            let sort = parse_sort_spec(opts.sort.as_deref());
            let mut rows = ctx
                .db
                .query_sets(&SetFilter {
                    states,
                    active_only: opts.active_only,
                    account_id: opts.account_id,
                    kind: set_kind,
                    search: opts.search.clone(),
                    limit: if sort.is_none() { opts.limit } else { None },
                })
                .await?;
            if let Some((field, desc)) = sort {
                match field.as_str() {
                    "id" | "set_id" => rows.sort_by_key(|s| s.set_id),
                    "kind" => rows
                        .sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.set_id.cmp(&b.set_id))),
                    "name" => rows.sort_by(|a, b| {
                        a.name
                            .to_lowercase()
                            .cmp(&b.name.to_lowercase())
                            .then_with(|| a.set_id.cmp(&b.set_id))
                    }),
                    "count" => rows.sort_by(|a, b| {
                        a.count.cmp(&b.count).then_with(|| a.set_id.cmp(&b.set_id))
                    }),
                    "state" => rows.sort_by(|a, b| {
                        a.state.cmp(&b.state).then_with(|| a.set_id.cmp(&b.set_id))
                    }),
                    other => {
                        return Err(AppError::usage(anyhow!(
                            "invalid sort field '{other}' for sets (expected: id, kind, name, count, state)"
                        )));
                    }
                }
                if desc {
                    rows.reverse();
                }
                if let Some(limit) = opts.limit {
                    rows.truncate(limit as usize);
                }
            }
            let count = rows.len();
            let rendered = match opts.format {
                OutputFormat::Table => render_table(
                    ["set_id", "kind", "name", "count", "state"],
                    rows.iter().map(|s| {
                        [
                            s.set_id.to_string(),
                            s.kind.clone(),
                            table_opts.format_cell(&s.name, 38),
                            s.count.to_string(),
                            format_state_badge(&s.state, table_opts.colored),
                        ]
                    }),
                    table_opts,
                ),
                OutputFormat::Json => render_json(&rows)?,
                OutputFormat::Jsonl => render_jsonl(&rows)?,
                OutputFormat::Tsv => render_tsv(
                    ["set_id", "kind", "name", "count", "state"],
                    rows.iter().map(|s| {
                        [
                            s.set_id.to_string(),
                            s.kind.clone(),
                            s.name.clone(),
                            s.count.to_string(),
                            s.state.clone(),
                        ]
                    }),
                ),
                OutputFormat::Ids => render_ids(rows.iter().map(|s| s.set_id.to_string())),
            };
            Ok((rendered, count))
        }
        ListKind::Up => {
            let mut states = parse_state_list::<UpState>(opts.state.as_deref())?;
            if opts.inactive_only {
                states.push(UpState::Inactive);
            }
            let sort = parse_sort_spec(opts.sort.as_deref());
            let mut rows = ctx
                .db
                .query_ups(&UpFilter {
                    states,
                    active_only: opts.active_only,
                    account_id: opts.account_id,
                    search: opts.search.clone(),
                    limit: if sort.is_none() { opts.limit } else { None },
                })
                .await?;
            if let Some((field, desc)) = sort {
                match field.as_str() {
                    "id" | "up_id" => rows.sort_by_key(|u| u.up_id),
                    "name" => rows.sort_by(|a, b| {
                        a.name
                            .to_lowercase()
                            .cmp(&b.name.to_lowercase())
                            .then_with(|| a.up_id.cmp(&b.up_id))
                    }),
                    "state" => rows
                        .sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.up_id.cmp(&b.up_id))),
                    other => {
                        return Err(AppError::usage(anyhow!(
                            "invalid sort field '{other}' for ups (expected: id, name, state)"
                        )));
                    }
                }
                if desc {
                    rows.reverse();
                }
                if let Some(limit) = opts.limit {
                    rows.truncate(limit as usize);
                }
            }
            let count = rows.len();
            let rendered = match opts.format {
                OutputFormat::Table => render_table(
                    ["up_id", "name", "state"],
                    rows.iter().map(|u| {
                        [
                            u.up_id.to_string(),
                            table_opts.format_cell(&u.name, 30),
                            format_state_badge(&u.state, table_opts.colored),
                        ]
                    }),
                    table_opts,
                ),
                OutputFormat::Json => render_json(&rows)?,
                OutputFormat::Jsonl => render_jsonl(&rows)?,
                OutputFormat::Tsv => render_tsv(
                    ["up_id", "name", "state"],
                    rows.iter()
                        .map(|u| [u.up_id.to_string(), u.name.clone(), u.state.clone()]),
                ),
                OutputFormat::Ids => render_ids(rows.iter().map(|u| u.up_id.to_string())),
            };
            Ok((rendered, count))
        }
        ListKind::Media => {
            if opts.inactive_only {
                return Err(AppError::usage(anyhow!(
                    "--inactive is not supported for medias; use --state <STATE>"
                )));
            }
            let states = parse_state_list::<MediaState>(opts.state.as_deref())?;
            let (sort, desc) = match parse_sort_spec(opts.sort.as_deref()) {
                None => (MediaSort::Id, false),
                Some((field, desc)) => {
                    let s = match field.as_str() {
                        "id" => MediaSort::Id,
                        "title" => MediaSort::Title,
                        "state" => MediaSort::State,
                        "updated" | "updated_at" => MediaSort::UpdatedAt,
                        other => {
                            return Err(AppError::usage(anyhow!(
                                "invalid sort field '{other}' for medias (expected: id, title, state, updated)"
                            )));
                        }
                    };
                    (s, desc)
                }
            };
            let media_type = match opts.media_type.as_deref() {
                Some(raw) => {
                    let parsed = crate::state::MediaType::from_str(raw)
                        .map_err(|e| AppError::usage(anyhow!("{e}")))?;
                    Some(parsed.to_string())
                }
                None => None,
            };
            let rows = ctx
                .db
                .query_medias(&MediaFilter {
                    states,
                    set_id: opts.set_id,
                    up_id: opts.up_id,
                    account_id: opts.account_id,
                    active_only: opts.active_only,
                    media_type,
                    search: opts.search.clone(),
                    sort,
                    desc,
                    limit: opts.limit,
                })
                .await?;
            let count = rows.len();
            let rendered = match opts.format {
                OutputFormat::Table => render_table(
                    ["id", "bvid", "title", "type", "state"],
                    rows.iter().map(|m| {
                        [
                            m.id.to_string(),
                            m.bv_id.clone(),
                            table_opts.format_cell(&m.title, 64),
                            m.r#type.clone(),
                            format_state_badge(&m.state, table_opts.colored),
                        ]
                    }),
                    table_opts,
                ),
                OutputFormat::Json => render_json(&rows)?,
                OutputFormat::Jsonl => render_jsonl(&rows)?,
                OutputFormat::Tsv => render_tsv(
                    ["id", "bvid", "title", "type", "state"],
                    rows.iter().map(|m| {
                        [
                            m.id.to_string(),
                            m.bv_id.clone(),
                            m.title.clone(),
                            m.r#type.clone(),
                            m.state.clone(),
                        ]
                    }),
                ),
                OutputFormat::Ids => render_ids(rows.iter().map(|m| m.id.to_string())),
            };
            Ok((rendered, count))
        }
    }
}

/// Execute `fav list <kind>` and print the formatted output to `stdout`.
pub async fn list(ctx: &AppContext, kind: ListKind, opts: ListOptions) -> AppResult<()> {
    let (rendered, count) = render_list(ctx, kind, &opts).await?;
    if !rendered.is_empty() || matches!(opts.format, OutputFormat::Json | OutputFormat::Table) {
        write_stdout_line(&rendered).map_err(AppError::runtime)?;
    }
    if count == 0 && opts.format == OutputFormat::Table {
        let hint = match kind {
            ListKind::Account => {
                "Run `fav auth login` or `fav auth usecookies --stdin` to add an account."
            }
            ListKind::Set => "Run `fav fetch` to discover favorite sets.",
            ListKind::Up => "Run `fav fetch` to discover followed UPs.",
            ListKind::Media => "Activate a set or UP (`fav activate set`) and run `fav fetch`.",
        };
        ctx.hint(hint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        api::BiliClientConfig,
        config::{Config, Workspace},
        db::Db,
        entity::set,
        error::ExitKind,
        muxer::test_support::MockMuxer,
    };

    #[tokio::test]
    async fn list_sets_displays_and_filters_by_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_sets([
            set::Model {
                set_id: 1,
                name: "My Created".to_owned(),
                count: 5,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(10),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 2,
                name: "Other Folder".to_owned(),
                count: 3,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Collected.to_string(),
                owner_mid: Some(20),
                sync_complete: false,
                resume_page: None,
            },
            set::Model {
                set_id: 3,
                name: "Course Season".to_owned(),
                count: 12,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Season.to_string(),
                owner_mid: Some(30),
                sync_complete: false,
                resume_page: None,
            },
        ])
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );

        let (tsv, count) = render_list(
            &ctx,
            ListKind::Set,
            &ListOptions {
                format: OutputFormat::Tsv,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(count, 3);
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines[0], "set_id\tkind\tname\tcount\tstate");
        assert!(lines[1].starts_with("1\tCreated\tMy Created\t5\tActive"));
        assert!(lines[2].starts_with("2\tCollected\tOther Folder\t3\tInactive"));
        assert!(lines[3].starts_with("3\tSeason\tCourse Season\t12\tInactive"));

        let (table, season_count) = render_list(
            &ctx,
            ListKind::Set,
            &ListOptions {
                format: OutputFormat::Table,
                kind: Some("season".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(season_count, 1);
        assert!(table.contains("kind"));
        assert!(table.contains("Season"));
        assert!(table.contains("Course Season"));
        assert!(!table.contains("My Created"));

        let err = render_list(
            &ctx,
            ListKind::Set,
            &ListOptions {
                kind: Some("bogus".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
    }
}
