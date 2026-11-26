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

            if s.accounts.total == 0 {
                ctx.hint("Run `fav auth login` or `fav auth usecookies --stdin` to sign in.");
            } else if s.sets.active == 0 && s.ups.active == 0 {
                ctx.hint("No active sets or UPs. Run `fav fetch`, then `fav activate set` (or `fav activate up`).");
            } else if s.medias.pending > 0 {
                ctx.hint(&format!(
                    "{} media(s) pending. Run `fav pull` (or `fav sync`) to download.",
                    s.medias.pending
                ));
            } else if s.medias.failed > 0 {
                ctx.hint(&format!(
                    "{} media(s) failed. Run `fav pull --retry-failed` or `fav media reset --failed`.",
                    s.medias.failed
                ));
            }
        }
    }

    Ok(())
}
