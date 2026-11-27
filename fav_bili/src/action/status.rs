use serde::Serialize;

use crate::{
    context::AppContext,
    db::StatusSummary,
    error::{AppError, AppResult},
    state::MediaState,
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

pub async fn status(ctx: &AppContext, format: OutputFormat, top: usize) -> AppResult<()> {
    let summary = ctx.db.status_summary(top).await?;
    let report = StatusReport {
        workspace: ctx.workspace.root().display().to_string(),
        db_path: ctx.workspace.db_path().display().to_string(),
        output_dir: ctx.effective_output_dir(None).display().to_string(),
        summary,
    };

    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&report).map_err(AppError::runtime)?;
            write_stdout_line(&json).map_err(AppError::runtime)?;
        }
        OutputFormat::Jsonl => {
            let jsonl = serde_json::to_string(&report).map_err(AppError::runtime)?;
            write_stdout_line(&jsonl).map_err(AppError::runtime)?;
        }
        OutputFormat::Ids => {
            let ids = render_ids(report.summary.top_ups.iter().map(|u| u.up_id.to_string()));
            if !ids.is_empty() {
                write_stdout_line(&ids).map_err(AppError::runtime)?;
            }
        }
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
                ["medias", "downloading", &s.medias.downloading.to_string()],
                ["medias", "completed", &s.medias.completed.to_string()],
                ["medias", "failed", &s.medias.failed.to_string()],
                ["medias", "expired", &s.medias.expired.to_string()],
                [
                    "medias",
                    "permission_denied",
                    &s.medias.permission_denied.to_string(),
                ],
                ["medias", "skipped", &s.medias.skipped.to_string()],
            ];
            let rendered = render_tsv(["section", "metric", "value"], rows);
            write_stdout_line(&rendered).map_err(AppError::runtime)?;
        }
        OutputFormat::Table => {
            let table_opts = TableOptions::detect(false, ctx.runtime.color);
            let c = table_opts.colored;
            let s = &report.summary;

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
                            "{}: {}, {}: {}, {}: {}, {}: {}, {}: {}, {}: {}",
                            format_state_badge("Pending", c),
                            s.medias.pending,
                            format_state_badge("Completed", c),
                            s.medias.completed,
                            format_state_badge("Failed", c),
                            s.medias.failed,
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

            if !write_stdout_line(&out).map_err(AppError::runtime)? {
                return Ok(());
            }

            if let Some(hint) = build_status_hint(ctx, s).await {
                ctx.hint(&hint);
            }
        }
    }

    Ok(())
}

async fn build_status_hint(ctx: &AppContext, s: &StatusSummary) -> Option<String> {
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
    let active_pending = ctx.db.count_active_pending_medias().await.unwrap_or(0);
    if active_pending > 0 {
        return Some(format!(
            "Run `fav pull` (or `fav sync`) to download {active_pending} active pending media(s)."
        ));
    }
    let active_failed = ctx
        .db
        .count_active_medias_in_state(MediaState::Failed)
        .await
        .unwrap_or(0);
    if active_failed > 0 {
        return Some(format!(
            "{active_failed} active media(s) failed. Run `fav pull --retry-failed` or `fav media reset --failed`."
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
        state::{AccountState, MediaType, SetKind, SetState},
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
        let hint = build_status_hint(&ctx, &summary).await.unwrap();
        assert!(!hint.contains("fav pull"), "unexpected hint: {hint}");
        assert!(hint.contains("1 pending media(s) belong to inactive sets/UPs"));

        // Mark inactive media as Failed -> must NOT tell user to run `fav pull --retry-failed`!
        ctx.db.set_media_state(2, MediaState::Failed).await.unwrap();
        let summary_failed = ctx.db.status_summary(0).await.unwrap();
        let hint_failed = build_status_hint(&ctx, &summary_failed).await.unwrap();
        assert!(
            !hint_failed.contains("fav pull"),
            "unexpected failed hint: {hint_failed}"
        );
        assert!(hint_failed.contains("1 failed media(s) belong to inactive sets/UPs"));

        // Activate set 20 -> active failed -> suggests `fav pull --retry-failed`!
        ctx.db.activate_set(20).await.unwrap();
        let summary_active_failed = ctx.db.status_summary(0).await.unwrap();
        let hint_active_failed = build_status_hint(&ctx, &summary_active_failed)
            .await
            .unwrap();
        assert!(hint_active_failed.contains("1 active media(s) failed"));
        assert!(hint_active_failed.contains("fav pull --retry-failed"));

        // Reset to Pending -> active pending -> suggests `fav pull`!
        ctx.db
            .set_media_state(2, MediaState::Pending)
            .await
            .unwrap();
        let summary_active_pending = ctx.db.status_summary(0).await.unwrap();
        let hint_active_pending = build_status_hint(&ctx, &summary_active_pending)
            .await
            .unwrap();
        assert!(
            hint_active_pending
                .contains("Run `fav pull` (or `fav sync`) to download 1 active pending media(s)")
        );
    }
}
