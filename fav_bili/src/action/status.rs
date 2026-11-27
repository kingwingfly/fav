use serde::Serialize;

use crate::{
    context::AppContext,
    db::StatusSummary,
    error::{AppError, AppResult},
    table::{
        OutputFormat, TableOptions, format_state_badge, render_ids, render_tsv, table,
        write_stdout_line,
    },
};

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub workspace: String,
    pub db_path: String,
    pub output_dir: String,
    #[serde(flatten)]
    pub summary: StatusSummary,
}

fn format_status_report(
    report: &StatusReport,
    format: OutputFormat,
    table_opts: TableOptions,
) -> AppResult<String> {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(report).map_err(AppError::runtime),
        OutputFormat::Jsonl => serde_json::to_string(report).map_err(AppError::runtime),
        OutputFormat::Ids => Ok(render_ids(
            report.summary.top_ups.iter().map(|u| u.up_id.to_string()),
        )),
        OutputFormat::Tsv => {
            let s = &report.summary;
            let rows = [
                ["workspace", "root", &report.workspace],
                ["workspace", "db_path", &report.db_path],
                ["workspace", "output_dir", &report.output_dir],
                ["accounts", "total", &s.accounts.total.to_string()],
                ["accounts", "active", &s.accounts.active.to_string()],
                ["accounts", "inactive", &s.accounts.inactive.to_string()],
                ["accounts", "expired", &s.accounts.expired.to_string()],
                ["sets", "total", &s.sets.total.to_string()],
                ["sets", "active", &s.sets.active.to_string()],
                ["sets", "inactive", &s.sets.inactive.to_string()],
                ["sets", "unreachable", &s.sets.unreachable.to_string()],
                ["ups", "total", &s.ups.total.to_string()],
                ["ups", "active", &s.ups.active.to_string()],
                ["ups", "inactive", &s.ups.inactive.to_string()],
                ["ups", "deactivated", &s.ups.deactivated.to_string()],
                ["medias", "total", &s.medias.total.to_string()],
                ["medias", "pending", &s.medias.pending.to_string()],
                [
                    "medias",
                    "active_pending",
                    &s.medias.active_pending.to_string(),
                ],
                ["medias", "downloading", &s.medias.downloading.to_string()],
                ["medias", "completed", &s.medias.completed.to_string()],
                ["medias", "failed", &s.medias.failed.to_string()],
                [
                    "medias",
                    "active_failed",
                    &s.medias.active_failed.to_string(),
                ],
                ["medias", "expired", &s.medias.expired.to_string()],
                [
                    "medias",
                    "permission_denied",
                    &s.medias.permission_denied.to_string(),
                ],
                ["medias", "skipped", &s.medias.skipped.to_string()],
            ];
            Ok(render_tsv(["section", "metric", "value"], rows))
        }
        OutputFormat::Table => Ok(render_status_table(report, table_opts)),
    }
}

pub async fn status(ctx: &AppContext, format: OutputFormat, top: usize) -> AppResult<()> {
    let summary = ctx.db.status_summary(top).await?;
    let report = StatusReport {
        workspace: ctx.workspace.root().display().to_string(),
        db_path: ctx.workspace.db_path().display().to_string(),
        output_dir: ctx.effective_output_dir(None).display().to_string(),
        summary,
    };

    let table_opts = TableOptions::detect(false, ctx.runtime.color);
    let rendered = format_status_report(&report, format, table_opts)?;

    match format {
        OutputFormat::Ids => {
            if !rendered.is_empty() {
                write_stdout_line(&rendered).map_err(AppError::runtime)?;
            }
        }
        OutputFormat::Table => {
            if !write_stdout_line(&rendered).map_err(AppError::runtime)? {
                return Ok(());
            }
            if let Some(hint) = build_status_hint(&report.summary) {
                ctx.hint(&hint);
            }
        }
        OutputFormat::Json | OutputFormat::Jsonl | OutputFormat::Tsv => {
            write_stdout_line(&rendered).map_err(AppError::runtime)?;
        }
    }

    Ok(())
}

fn render_status_table(report: &StatusReport, table_opts: TableOptions) -> String {
    let c = table_opts.colored;
    let s = &report.summary;

    let pending_display = if s.medias.pending > 0 && s.medias.active_pending != s.medias.pending {
        format!("{} ({} active)", s.medias.pending, s.medias.active_pending)
    } else {
        s.medias.pending.to_string()
    };
    let failed_display = if s.medias.failed > 0 && s.medias.active_failed != s.medias.failed {
        format!("{} ({} active)", s.medias.failed, s.medias.active_failed)
    } else {
        s.medias.failed.to_string()
    };

    let mut overview = table(
        ["resource", "total", "breakdown"],
        [
            [
                "Accounts".to_owned(),
                s.accounts.total.to_string(),
                format!(
                    "{}: {}, {}: {}, {}: {}",
                    format_state_badge("Active", c),
                    s.accounts.active,
                    format_state_badge("Inactive", c),
                    s.accounts.inactive,
                    format_state_badge("Expired", c),
                    s.accounts.expired,
                ),
            ],
            [
                "Sets".to_owned(),
                s.sets.total.to_string(),
                format!(
                    "{}: {}, {}: {}, {}: {}",
                    format_state_badge("Active", c),
                    s.sets.active,
                    format_state_badge("Inactive", c),
                    s.sets.inactive,
                    format_state_badge("Unreachable", c),
                    s.sets.unreachable,
                ),
            ],
            [
                "UPs".to_owned(),
                s.ups.total.to_string(),
                format!(
                    "{}: {}, {}: {}, {}: {}",
                    format_state_badge("Active", c),
                    s.ups.active,
                    format_state_badge("Inactive", c),
                    s.ups.inactive,
                    format_state_badge("Deactivated", c),
                    s.ups.deactivated,
                ),
            ],
            [
                "Medias".to_owned(),
                s.medias.total.to_string(),
                format!(
                    "{}: {}, {}: {}, {}: {}, {}: {}, {}: {}, {}: {}, {}: {}",
                    format_state_badge("Pending", c),
                    pending_display,
                    format_state_badge("Downloading", c),
                    s.medias.downloading,
                    format_state_badge("Completed", c),
                    s.medias.completed,
                    format_state_badge("Failed", c),
                    failed_display,
                    format_state_badge("Expired", c),
                    s.medias.expired,
                    format_state_badge("PermissionDenied", c),
                    s.medias.permission_denied,
                    format_state_badge("Skipped", c),
                    s.medias.skipped,
                ),
            ],
        ],
    );
    if !table_opts.wide
        && let Some(max_width) = table_opts.term_width
    {
        overview.fit_to_width(max_width);
    }

    let mut out = format!(
        "Workspace : {}\nDatabase  : {}\nOutput Dir: {}\n\n{overview}",
        report.workspace, report.db_path, report.output_dir
    );

    if !s.top_ups.is_empty() {
        let mut top_tbl = table(
            ["up_id", "name", "medias"],
            s.top_ups.iter().map(|u| {
                [
                    u.up_id.to_string(),
                    table_opts.format_cell(&u.name, 30),
                    u.media_count.to_string(),
                ]
            }),
        );
        if !table_opts.wide
            && let Some(max_width) = table_opts.term_width
        {
            top_tbl.fit_to_width(max_width);
        }
        out.push_str("\n\nTop UPs:\n");
        out.push_str(&top_tbl.to_string());
    }

    out
}

fn build_status_hint(s: &StatusSummary) -> Option<String> {
    if s.accounts.total == 0 {
        return Some(
            "Run `fav auth login` or `fav auth usecookies --stdin` to sign in.".to_owned(),
        );
    }
    if s.accounts.active == 0 {
        return Some(
            "No active accounts. Run `fav activate account` or `fav auth login` first.".to_owned(),
        );
    }
    if s.sets.active == 0 && s.ups.active == 0 {
        return Some(
            "No active sets or UPs. Run `fav fetch`, then `fav activate set` (or `fav activate up`)."
                .to_owned(),
        );
    }
    if s.medias.active_pending > 0 {
        return Some(format!(
            "Run `fav pull` (or `fav sync`) to download {} active pending media(s).",
            s.medias.active_pending
        ));
    }
    if s.medias.active_failed > 0 {
        return Some(format!(
            "{} active media(s) failed. Run `fav pull --retry-failed` or `fav media reset --failed`.",
            s.medias.active_failed
        ));
    }
    if s.medias.pending > 0 {
        return Some(format!(
            "{} pending media(s) belong to inactive sets/UPs; activate them with `fav activate set` / `fav activate up` before pulling.",
            s.medias.pending
        ));
    }
    if s.medias.failed > 0 {
        return Some(format!(
            "{} failed media(s) belong to inactive sets/UPs; activate them with `fav activate set` / `fav activate up` before retrying.",
            s.medias.failed
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        api::BiliClientConfig,
        config::{Config, Workspace},
        db::Db,
        entity::{account, media, media_set, set, set_account},
        muxer::test_support::MockMuxer,
        state::{AccountState, MediaState, MediaType, SetKind, SetState},
    };

    #[tokio::test]
    async fn status_hint_distinguishes_active_and_inactive_pending_and_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=s".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_sets([
            set::Model {
                set_id: 10,
                name: "ActiveSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 20,
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
                set_id: 20,
                account_id: 1,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 1,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "ActiveCompleted".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 2,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "InactivePending".to_owned(),
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
            media_set::Model { id: 1, set_id: 10 },
            media_set::Model { id: 2, set_id: 20 },
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

        // Only inactive pending -> must NOT tell user to run `fav pull`!
        let summary = ctx.db.status_summary(0).await.unwrap();
        assert_eq!(summary.medias.pending, 1);
        assert_eq!(summary.medias.active_pending, 0);
        let hint = build_status_hint(&summary).unwrap();
        assert!(!hint.contains("fav pull"), "unexpected hint: {hint}");
        assert!(hint.contains("1 pending media(s) belong to inactive sets/UPs"));

        // Mark inactive media as Failed -> must NOT tell user to run `fav pull --retry-failed`!
        ctx.db.set_media_state(2, MediaState::Failed).await.unwrap();
        let summary_failed = ctx.db.status_summary(0).await.unwrap();
        assert_eq!(summary_failed.medias.failed, 1);
        assert_eq!(summary_failed.medias.active_failed, 0);
        let hint_failed = build_status_hint(&summary_failed).unwrap();
        assert!(
            !hint_failed.contains("fav pull"),
            "unexpected failed hint: {hint_failed}"
        );
        assert!(hint_failed.contains("1 failed media(s) belong to inactive sets/UPs"));

        // Activate set 20 -> active failed -> suggests `fav pull --retry-failed`!
        ctx.db.activate_set(20).await.unwrap();
        let summary_active_failed = ctx.db.status_summary(0).await.unwrap();
        assert_eq!(summary_active_failed.medias.active_failed, 1);
        let hint_active_failed = build_status_hint(&summary_active_failed).unwrap();
        assert!(hint_active_failed.contains("1 active media(s) failed"));
        assert!(hint_active_failed.contains("fav pull --retry-failed"));

        // Reset to Pending -> active pending -> suggests `fav pull`!
        ctx.db
            .set_media_state(2, MediaState::Pending)
            .await
            .unwrap();
        let summary_active_pending = ctx.db.status_summary(0).await.unwrap();
        assert_eq!(summary_active_pending.medias.active_pending, 1);
        let hint_active_pending = build_status_hint(&summary_active_pending).unwrap();
        assert!(
            hint_active_pending
                .contains("Run `fav pull` (or `fav sync`) to download 1 active pending media(s)")
        );
    }

    #[tokio::test]
    async fn status_output_surfaces_active_vs_inactive_pending_and_failed_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=s".to_owned(),
            state: AccountState::Active.to_string(),
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
                set_id: 20,
                name: "InactiveSet".to_owned(),
                count: 2,
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
                set_id: 20,
                account_id: 1,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 1,
                bv_id: "BV1active0001".to_owned(),
                title: "ActivePending".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 2,
                bv_id: "BV1active0002".to_owned(),
                title: "ActiveFailed".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Failed.to_string(),
                retry_count: 1,
                last_error: Some("err".to_owned()),
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 1, set_id: 10 },
            media_set::Model { id: 2, set_id: 10 },
        ])
        .await
        .unwrap();

        let plain_opts = TableOptions::detect(true, crate::table::ColorChoice::Never);

        // Case 1: All pending & failed are active -> clean `Pending: 1`, `Failed: 1` with no noise
        let all_active_report = StatusReport {
            workspace: ws.root().display().to_string(),
            db_path: ws.db_path().display().to_string(),
            output_dir: ws.root().display().to_string(),
            summary: db.status_summary(0).await.unwrap(),
        };
        let table_all_active =
            format_status_report(&all_active_report, OutputFormat::Table, plain_opts).unwrap();
        assert!(
            table_all_active.contains("Pending: 1,"),
            "unexpected all-active table:\n{table_all_active}"
        );
        assert!(
            table_all_active.contains("Failed: 1,"),
            "unexpected all-active table:\n{table_all_active}"
        );
        assert!(
            !table_all_active.contains(" active)"),
            "all-active table must not add active parenthetical noise:\n{table_all_active}"
        );

        let tsv_all_active =
            format_status_report(&all_active_report, OutputFormat::Tsv, plain_opts).unwrap();
        assert!(tsv_all_active.contains("medias\tpending\t1"));
        assert!(tsv_all_active.contains("medias\tactive_pending\t1"));
        assert!(tsv_all_active.contains("medias\tfailed\t1"));
        assert!(tsv_all_active.contains("medias\tactive_failed\t1"));

        // Case 2: Add inactive Pending and inactive Failed -> mixed active/inactive display
        db.upsert_medias([
            media::Model {
                id: 3,
                bv_id: "BV1inact00003".to_owned(),
                title: "InactivePending".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 4,
                bv_id: "BV1inact00004".to_owned(),
                title: "InactiveFailed".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Failed.to_string(),
                retry_count: 1,
                last_error: Some("err".to_owned()),
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 3, set_id: 20 },
            media_set::Model { id: 4, set_id: 20 },
        ])
        .await
        .unwrap();

        let mixed_report = StatusReport {
            workspace: ws.root().display().to_string(),
            db_path: ws.db_path().display().to_string(),
            output_dir: ws.root().display().to_string(),
            summary: db.status_summary(0).await.unwrap(),
        };
        let table_mixed =
            format_status_report(&mixed_report, OutputFormat::Table, plain_opts).unwrap();
        assert!(
            table_mixed.contains("Pending: 2 (1 active)"),
            "expected mixed Pending display:\n{table_mixed}"
        );
        assert!(
            table_mixed.contains("Failed: 2 (1 active)"),
            "expected mixed Failed display:\n{table_mixed}"
        );

        let tsv_mixed = format_status_report(&mixed_report, OutputFormat::Tsv, plain_opts).unwrap();
        assert!(tsv_mixed.contains("medias\tpending\t2"));
        assert!(tsv_mixed.contains("medias\tactive_pending\t1"));
        assert!(tsv_mixed.contains("medias\tfailed\t2"));
        assert!(tsv_mixed.contains("medias\tactive_failed\t1"));

        let json_mixed =
            format_status_report(&mixed_report, OutputFormat::Json, plain_opts).unwrap();
        let json_val: serde_json::Value = serde_json::from_str(&json_mixed).unwrap();
        assert_eq!(json_val["medias"]["pending"], 2);
        assert_eq!(json_val["medias"]["active_pending"], 1);
        assert_eq!(json_val["medias"]["failed"], 2);
        assert_eq!(json_val["medias"]["active_failed"], 1);

        let jsonl_mixed =
            format_status_report(&mixed_report, OutputFormat::Jsonl, plain_opts).unwrap();
        let jsonl_val: serde_json::Value = serde_json::from_str(&jsonl_mixed).unwrap();
        assert_eq!(jsonl_val["medias"]["active_pending"], 1);
        assert_eq!(jsonl_val["medias"]["active_failed"], 1);
    }

    #[tokio::test]
    async fn status_table_includes_downloading_breakdown_matching_total() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        let medias = [
            (1, "BV17x411w7KC", MediaState::Pending),
            (2, "BV1Q541167Qg", MediaState::Downloading),
            (3, "BV1xx411c7mD", MediaState::Downloading),
            (4, "BV1ab411c7mD", MediaState::Completed),
            (5, "BV1cd411c7mD", MediaState::Failed),
            (6, "BV1ef411c7mD", MediaState::Expired),
            (7, "BV1gh411c7mD", MediaState::PermissionDenied),
            (8, "BV1ij411c7mD", MediaState::Skipped),
        ]
        .into_iter()
        .map(|(id, bv_id, state)| media::Model {
            id,
            bv_id: bv_id.to_owned(),
            title: format!("Media {id}"),
            r#type: MediaType::Video.to_string(),
            state: state.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        });
        db.upsert_medias(medias).await.unwrap();

        let summary = db.status_summary(0).await.unwrap();
        assert_eq!(summary.medias.total, 8);
        assert_eq!(summary.medias.downloading, 2);

        let report = StatusReport {
            workspace: ws.root().display().to_string(),
            db_path: ws.db_path().display().to_string(),
            output_dir: ws.root().display().to_string(),
            summary,
        };
        let rendered = render_status_table(
            &report,
            TableOptions {
                wide: true,
                is_tty: false,
                colored: false,
                term_width: None,
            },
        );
        assert!(
            rendered.contains(
                "Pending: 1 (0 active), Downloading: 2, Completed: 1, Failed: 1 (0 active), Expired: 1, PermissionDenied: 1, Skipped: 1"
            ),
            "unexpected status table:\n{rendered}"
        );
    }
}
