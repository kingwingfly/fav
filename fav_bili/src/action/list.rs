use std::str::FromStr as _;

use anyhow::anyhow;
use serde::Serialize;

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
    pub followed_only: bool,
    pub media_type: Option<String>,
    pub search: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize)]
struct UpRow {
    up_id: i64,
    name: String,
    state: String,
    followed: bool,
}

impl ListOptions {
    /// Returns `true` when any filtering option is active.
    pub fn has_filters(&self) -> bool {
        self.state.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.kind.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.set_id.is_some()
            || self.up_id.is_some()
            || self.account_id.is_some()
            || self.active_only
            || self.inactive_only
            || self.followed_only
            || self
                .media_type
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            || self.search.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
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
    let has_state = opts.state.as_deref().is_some_and(|s| !s.trim().is_empty());
    if !matches!(kind, ListKind::Media) && has_state && (opts.active_only || opts.inactive_only) {
        let flag = if opts.active_only {
            "--active"
        } else {
            "--inactive"
        };
        return Err(AppError::usage(anyhow!(
            "--state cannot be combined with {flag}"
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
                    followed_only: opts.followed_only,
                    account_id: opts.account_id,
                    search: opts.search.clone(),
                    limit: if sort.is_none() { opts.limit } else { None },
                })
                .await?;
            let followed_ids = ctx.db.active_account_up_ids().await?;
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
                    "followed" => rows.sort_by(|a, b| {
                        followed_ids
                            .contains(&a.up_id)
                            .cmp(&followed_ids.contains(&b.up_id))
                            .then_with(|| a.up_id.cmp(&b.up_id))
                    }),
                    other => {
                        return Err(AppError::usage(anyhow!(
                            "invalid sort field '{other}' for ups (expected: id, name, state, followed)"
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
            let up_rows: Vec<UpRow> = rows
                .iter()
                .map(|u| UpRow {
                    up_id: u.up_id,
                    name: u.name.clone(),
                    state: u.state.clone(),
                    followed: followed_ids.contains(&u.up_id),
                })
                .collect();
            let rendered = match opts.format {
                OutputFormat::Table => render_table(
                    ["up_id", "name", "state", "followed"],
                    up_rows.iter().map(|u| {
                        [
                            u.up_id.to_string(),
                            table_opts.format_cell(&u.name, 30),
                            format_state_badge(&u.state, table_opts.colored),
                            if u.followed { "yes" } else { "no" }.to_owned(),
                        ]
                    }),
                    table_opts,
                ),
                OutputFormat::Json => render_json(&up_rows)?,
                OutputFormat::Jsonl => render_jsonl(&up_rows)?,
                OutputFormat::Tsv => render_tsv(
                    ["up_id", "name", "state", "followed"],
                    up_rows.iter().map(|u| {
                        [
                            u.up_id.to_string(),
                            u.name.clone(),
                            u.state.clone(),
                            if u.followed { "yes" } else { "no" }.to_owned(),
                        ]
                    }),
                ),
                OutputFormat::Ids => render_ids(up_rows.iter().map(|u| u.up_id.to_string())),
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

fn empty_list_hint(kind: ListKind, opts: &ListOptions, count: usize) -> Option<&'static str> {
    if count != 0 || opts.format != OutputFormat::Table || opts.has_filters() {
        return None;
    }
    Some(match kind {
        ListKind::Account => {
            "Run `fav auth login` or `fav auth usecookies --stdin` to add an account."
        }
        ListKind::Set => "Run `fav fetch` to discover favorite sets.",
        ListKind::Up => "Run `fav fetch` to discover followed UPs.",
        ListKind::Media => "Activate a set or UP (`fav activate set`) and run `fav fetch`.",
    })
}

/// Execute `fav list <kind>` and print the formatted output to `stdout`.
pub async fn list(ctx: &AppContext, kind: ListKind, opts: ListOptions) -> AppResult<()> {
    let (rendered, count) = render_list(ctx, kind, &opts).await?;
    if !rendered.is_empty() || matches!(opts.format, OutputFormat::Json | OutputFormat::Table) {
        write_stdout_line(&rendered).map_err(AppError::runtime)?;
    }
    if let Some(hint) = empty_list_hint(kind, &opts, count) {
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

    const ALL_KINDS: [ListKind; 4] = [
        ListKind::Account,
        ListKind::Set,
        ListKind::Up,
        ListKind::Media,
    ];

    #[test]
    fn empty_list_hint_returned_for_unfiltered_empty_table() {
        let default_opts = ListOptions::default();
        assert!(!default_opts.has_filters());
        assert_eq!(
            empty_list_hint(ListKind::Account, &default_opts, 0),
            Some("Run `fav auth login` or `fav auth usecookies --stdin` to add an account.")
        );
        assert_eq!(
            empty_list_hint(ListKind::Set, &default_opts, 0),
            Some("Run `fav fetch` to discover favorite sets.")
        );
        assert_eq!(
            empty_list_hint(ListKind::Up, &default_opts, 0),
            Some("Run `fav fetch` to discover followed UPs.")
        );
        assert_eq!(
            empty_list_hint(ListKind::Media, &default_opts, 0),
            Some("Activate a set or UP (`fav activate set`) and run `fav fetch`.")
        );

        let non_filter_opts = ListOptions {
            wide: true,
            state: Some("   ".to_owned()),
            kind: Some(String::new()),
            media_type: Some(" \t ".to_owned()),
            search: Some("  ".to_owned()),
            sort: Some("id".to_owned()),
            limit: Some(10),
            ..Default::default()
        };
        assert!(!non_filter_opts.has_filters());
        for kind in ALL_KINDS {
            assert!(empty_list_hint(kind, &non_filter_opts, 0).is_some());
        }
    }

    #[test]
    fn empty_list_hint_suppressed_when_filters_active() {
        let filtered_cases = [
            ListOptions {
                state: Some("pending".to_owned()),
                ..Default::default()
            },
            ListOptions {
                kind: Some("created".to_owned()),
                ..Default::default()
            },
            ListOptions {
                set_id: Some(1),
                ..Default::default()
            },
            ListOptions {
                up_id: Some(2),
                ..Default::default()
            },
            ListOptions {
                account_id: Some(3),
                ..Default::default()
            },
            ListOptions {
                active_only: true,
                ..Default::default()
            },
            ListOptions {
                inactive_only: true,
                ..Default::default()
            },
            ListOptions {
                followed_only: true,
                ..Default::default()
            },
            ListOptions {
                media_type: Some("video".to_owned()),
                ..Default::default()
            },
            ListOptions {
                search: Some("rust".to_owned()),
                ..Default::default()
            },
        ];

        for opts in &filtered_cases {
            assert!(opts.has_filters(), "expected has_filters() for {opts:?}");
            for kind in ALL_KINDS {
                assert_eq!(
                    empty_list_hint(kind, opts, 0),
                    None,
                    "expected no hint for {kind:?} with {opts:?}"
                );
            }
        }
    }

    #[test]
    fn empty_list_hint_suppressed_when_non_empty_or_non_table() {
        let default_opts = ListOptions::default();
        for kind in ALL_KINDS {
            assert_eq!(empty_list_hint(kind, &default_opts, 1), None);
        }

        for format in [
            OutputFormat::Json,
            OutputFormat::Jsonl,
            OutputFormat::Tsv,
            OutputFormat::Ids,
        ] {
            let opts = ListOptions {
                format,
                ..Default::default()
            };
            assert!(!opts.has_filters());
            for kind in ALL_KINDS {
                assert_eq!(
                    empty_list_hint(kind, &opts, 0),
                    None,
                    "expected no hint for {kind:?} in format {format:?}"
                );
            }
        }
    }

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

    #[tokio::test]
    async fn list_ups_shows_followed_status_and_filters_by_followed() {
        use crate::entity::{account, up, up_account};

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "ActiveUser".to_owned(),
            cookies: "SESSDATA=a".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "InactiveUser".to_owned(),
            cookies: "SESSDATA=b".to_owned(),
            state: AccountState::Inactive.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();

        db.upsert_ups([
            up::Model {
                up_id: 10,
                name: "FollowedCreator".to_owned(),
                state: UpState::Active.to_string(),
                sync_complete: false,
                resume_page: None,
            },
            up::Model {
                up_id: 20,
                name: "InactiveAcctFollowed".to_owned(),
                state: UpState::Inactive.to_string(),
                sync_complete: false,
                resume_page: None,
            },
            up::Model {
                up_id: 30,
                name: "UnfollowedFavCreator".to_owned(),
                state: UpState::Inactive.to_string(),
                sync_complete: false,
                resume_page: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_up_accounts([
            up_account::Model {
                up_id: 10,
                account_id: 1,
            },
            up_account::Model {
                up_id: 20,
                account_id: 2,
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

        // Unfiltered TSV shows all 3 with followed yes/no column
        let (tsv, count) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Tsv,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(count, 3);
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines[0], "up_id\tname\tstate\tfollowed");
        assert_eq!(lines[1], "10\tFollowedCreator\tActive\tyes");
        assert_eq!(lines[2], "20\tInactiveAcctFollowed\tInactive\tno");
        assert_eq!(lines[3], "30\tUnfollowedFavCreator\tInactive\tno");

        // Unfiltered Table shows followed header and yes/no values
        let (table, table_count) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Table,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(table_count, 3);
        assert!(table.contains("followed"));
        assert!(table.contains("FollowedCreator"));
        assert!(table.contains("UnfollowedFavCreator"));
        assert!(table.contains("yes"));
        assert!(table.contains("no"));

        // JSON and JSONL serialize followed: bool alongside up_id/name/state
        let (json_str, _) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Json,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let json_val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let arr = json_val.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["up_id"], 10);
        assert_eq!(arr[0]["name"], "FollowedCreator");
        assert_eq!(arr[0]["state"], "Active");
        assert_eq!(arr[0]["followed"], true);
        assert_eq!(arr[1]["up_id"], 20);
        assert_eq!(arr[1]["followed"], false);
        assert_eq!(arr[2]["up_id"], 30);
        assert_eq!(arr[2]["followed"], false);

        let (jsonl_str, _) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Jsonl,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let jsonl_rows: Vec<serde_json::Value> = jsonl_str
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(jsonl_rows.len(), 3);
        assert_eq!(jsonl_rows[0]["followed"], true);
        assert_eq!(jsonl_rows[2]["followed"], false);

        // --followed filters out unfollowed favorite-set creators and inactive-account follows
        let (followed_tsv, followed_count) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Tsv,
                followed_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(followed_count, 1);
        let f_lines: Vec<&str> = followed_tsv.lines().collect();
        assert_eq!(f_lines.len(), 2);
        assert_eq!(f_lines[0], "up_id\tname\tstate\tfollowed");
        assert_eq!(f_lines[1], "10\tFollowedCreator\tActive\tyes");

        // --sort followed puts unfollowed (false) first, then followed (true)
        let (sort_asc_ids, sort_asc_count) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Ids,
                sort: Some("followed".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(sort_asc_count, 3);
        assert_eq!(sort_asc_ids, "20\n30\n10");

        // --sort -followed puts followed (true) first, then unfollowed (false)
        let (sort_desc_ids, sort_desc_count) = render_list(
            &ctx,
            ListKind::Up,
            &ListOptions {
                format: OutputFormat::Ids,
                sort: Some("-followed".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(sort_desc_count, 3);
        assert_eq!(sort_desc_ids, "10\n30\n20");
    }

    #[tokio::test]
    async fn list_rejects_state_combined_with_active_or_inactive_except_media_active() {
        use crate::{
            entity::{account, media, media_set, set_account, up},
            state::MediaType,
        };

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "ActiveUser".to_owned(),
            cookies: "SESSDATA=a".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "InactiveUser".to_owned(),
            cookies: "SESSDATA=b".to_owned(),
            state: AccountState::Inactive.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();

        db.upsert_sets([
            set::Model {
                set_id: 10,
                name: "ActiveSet".to_owned(),
                count: 2,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 11,
                name: "InactiveSet".to_owned(),
                count: 1,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 11,
                account_id: 1,
            },
        ])
        .await
        .unwrap();

        db.upsert_ups([
            up::Model {
                up_id: 100,
                name: "ActiveUp".to_owned(),
                state: UpState::Active.to_string(),
                sync_complete: false,
                resume_page: None,
            },
            up::Model {
                up_id: 101,
                name: "InactiveUp".to_owned(),
                state: UpState::Inactive.to_string(),
                sync_complete: false,
                resume_page: None,
            },
        ])
        .await
        .unwrap();

        db.upsert_medias([
            media::Model {
                id: 1000,
                bv_id: "BV1activePend".to_owned(),
                title: "Active Pending".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 1001,
                bv_id: "BV1activeDone".to_owned(),
                title: "Active Completed".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 1002,
                bv_id: "BV1inactPend".to_owned(),
                title: "Inactive Pending".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model {
                id: 1000,
                set_id: 10,
            },
            media_set::Model {
                id: 1001,
                set_id: 10,
            },
            media_set::Model {
                id: 1002,
                set_id: 11,
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

        // Account, Set, Up: combining --state with --active or --inactive MUST fail with ExitKind::Usage.
        for (kind, sample_state) in [
            (ListKind::Account, "expired"),
            (ListKind::Set, "unreachable"),
            (ListKind::Up, "deactivated"),
        ] {
            let err_active = render_list(
                &ctx,
                kind,
                &ListOptions {
                    state: Some(sample_state.to_owned()),
                    active_only: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err_active.kind(), ExitKind::Usage);
            assert!(
                err_active.to_string().contains("--state")
                    && err_active.to_string().contains("--active"),
                "unexpected error for {kind:?} --state --active: {err_active}"
            );

            let err_inactive = render_list(
                &ctx,
                kind,
                &ListOptions {
                    state: Some("active".to_owned()),
                    inactive_only: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err_inactive.kind(), ExitKind::Usage);
            assert!(
                err_inactive.to_string().contains("--state")
                    && err_inactive.to_string().contains("--inactive"),
                "unexpected error for {kind:?} --state --inactive: {err_inactive}"
            );

            // Non-conflicting filters continue to work cleanly.
            let (_, active_count) = render_list(
                &ctx,
                kind,
                &ListOptions {
                    format: OutputFormat::Ids,
                    active_only: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(active_count, 1, "active filter for {kind:?}");

            let (_, inactive_count) = render_list(
                &ctx,
                kind,
                &ListOptions {
                    format: OutputFormat::Ids,
                    inactive_only: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(inactive_count, 1, "inactive filter for {kind:?}");

            let (_, state_count) = render_list(
                &ctx,
                kind,
                &ListOptions {
                    format: OutputFormat::Ids,
                    state: Some("active".to_owned()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(state_count, 1, "state filter for {kind:?}");
        }

        // Media: --state pending --active MUST succeed and intersect MediaState with active-source scope!
        let (ids, count) = render_list(
            &ctx,
            ListKind::Media,
            &ListOptions {
                format: OutputFormat::Ids,
                state: Some("pending".to_owned()),
                active_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(ids, "1000");

        // Media: --inactive remains rejected with ExitKind::Usage even alongside --state.
        let media_inactive_err = render_list(
            &ctx,
            ListKind::Media,
            &ListOptions {
                state: Some("pending".to_owned()),
                inactive_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(media_inactive_err.kind(), ExitKind::Usage);
        assert!(media_inactive_err.to_string().contains("--inactive"));
    }
}
