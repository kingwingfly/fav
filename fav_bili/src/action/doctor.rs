use std::path::Path;

use anyhow::anyhow;
use console::style;
use serde::Serialize;

use crate::{
    context::AppContext,
    cookies::{extract_csrf, parse_cookie_header, validate_sessdata},
    error::{AppError, AppResult},
    table::{
        OutputFormat, TableOptions, render_ids, render_json, render_jsonl, render_table, render_tsv,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

impl CheckStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn badge(self, colored: bool) -> String {
        let label = match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        };
        if !colored {
            return label.to_owned();
        }
        match self {
            Self::Ok => style(label).force_styling(true).green().to_string(),
            Self::Warn => style(label).force_styling(true).yellow().to_string(),
            Self::Fail => style(label).force_styling(true).red().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

fn check_unix_permissions(fav_dir: &Path, db_path: &Path) -> DoctorCheck {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mut issues = Vec::new();
        if let Ok(meta) = std::fs::metadata(fav_dir) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                issues.push(format!(
                    "{} is {:04o} (expected 0700)",
                    fav_dir.display(),
                    mode
                ));
            }
        }
        if let Ok(meta) = std::fs::metadata(db_path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                issues.push(format!(
                    "{} is {:04o} (expected 0600)",
                    db_path.display(),
                    mode
                ));
            }
        }

        if issues.is_empty() {
            DoctorCheck {
                name: "permissions".into(),
                status: CheckStatus::Ok,
                detail: "owner-only (0700/0600)".into(),
                hint: None,
            }
        } else {
            DoctorCheck {
                name: "permissions".into(),
                status: CheckStatus::Warn,
                detail: issues.join("; "),
                hint: Some(format!(
                    "chmod 700 '{}' && chmod 600 '{}'",
                    fav_dir.display(),
                    db_path.display()
                )),
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (fav_dir, db_path);
        DoctorCheck {
            name: "permissions".into(),
            status: CheckStatus::Ok,
            detail: "skipped (non-Unix)".into(),
            hint: None,
        }
    }
}

pub async fn doctor(ctx: &AppContext, format: OutputFormat, offline: bool) -> AppResult<()> {
    let mut checks: Vec<DoctorCheck> = Vec::new();
    let mut non_auth_failed = false;
    let mut auth_failed = false;

    // 1. Workspace & SQLite integrity
    let db_path = ctx.workspace.db_path();
    match ctx.db.check_integrity().await {
        Ok(()) => checks.push(DoctorCheck {
            name: "database".into(),
            status: CheckStatus::Ok,
            detail: format!("{} (integrity ok)", db_path.display()),
            hint: None,
        }),
        Err(e) => {
            non_auth_failed = true;
            checks.push(DoctorCheck {
                name: "database".into(),
                status: CheckStatus::Fail,
                detail: format!("{}: {e:#}", db_path.display()),
                hint: Some("Restore from backup or inspect SQLite file.".into()),
            });
        }
    }

    // 2. Unix permissions on .fav and fav.db
    checks.push(check_unix_permissions(&ctx.workspace.fav_dir(), &db_path));

    // 3. FFmpeg availability
    match ctx.muxer.probe().await {
        Ok(version_line) => checks.push(DoctorCheck {
            name: "ffmpeg".into(),
            status: CheckStatus::Ok,
            detail: version_line,
            hint: None,
        }),
        Err(e) => {
            non_auth_failed = true;
            checks.push(DoctorCheck {
                name: "ffmpeg".into(),
                status: CheckStatus::Fail,
                detail: format!("{e:#}"),
                hint: Some(
                    "Install `ffmpeg` on PATH or set `ffmpeg_path` in `.fav/config.toml`.".into(),
                ),
            });
        }
    }

    // 4. DB-vs-disk drift (completed media pages + leftover temp .part files)
    let output_dir = ctx.effective_output_dir(None);
    match ctx.db.all_completed_pages().await {
        Ok(pages) => {
            let total = pages.len();
            let mut missing = 0usize;
            for page in &pages {
                let exists = page.local_path.as_deref().is_some_and(|raw| {
                    let p = Path::new(raw);
                    if p.is_absolute() {
                        p.is_file()
                    } else {
                        output_dir.join(p).is_file()
                    }
                });
                if !exists {
                    missing += 1;
                }
            }
            if missing == 0 {
                checks.push(DoctorCheck {
                    name: "disk_sync".into(),
                    status: CheckStatus::Ok,
                    detail: format!("{total} completed page(s) verified on disk"),
                    hint: None,
                });
            } else {
                checks.push(DoctorCheck {
                    name: "disk_sync".into(),
                    status: CheckStatus::Warn,
                    detail: format!(
                        "{missing}/{total} completed page file(s) missing under {}",
                        output_dir.display()
                    ),
                    hint: Some(
                        "Run `fav media reset --missing` to re-queue missing videos.".into(),
                    ),
                });
            }
        }
        Err(e) => {
            non_auth_failed = true;
            checks.push(DoctorCheck {
                name: "disk_sync".into(),
                status: CheckStatus::Fail,
                detail: format!("failed to query completed pages: {e:#}"),
                hint: None,
            });
        }
    }

    let tmp_dir = ctx.workspace.tmp_dir();
    if tmp_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&tmp_dir)
    {
        let part_count = entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".part"))
            })
            .count();
        if part_count > 0 {
            checks.push(DoctorCheck {
                name: "staging_tmp".into(),
                status: CheckStatus::Warn,
                detail: format!(
                    "{part_count} resumable .part file(s) in {}",
                    tmp_dir.display()
                ),
                hint: Some(
                    "Run `fav pull` to resume partial downloads or delete `.fav/tmp/*.part`."
                        .into(),
                ),
            });
        }
    }

    // 5. Accounts & cookies (offline capability check + optional live nav_info check)
    match ctx.db.all_accounts().await {
        Ok(accounts) if accounts.is_empty() => {
            checks.push(DoctorCheck {
                name: "accounts".into(),
                status: CheckStatus::Warn,
                detail: "no accounts configured".into(),
                hint: Some("Run `fav auth login` or `fav auth usecookies --stdin`.".into()),
            });
        }
        Ok(accounts) => {
            for acct in accounts {
                let check_name = format!("account:{} ({})", acct.account_id, acct.name);
                let cookie_map = parse_cookie_header(&acct.cookies);
                if let Err(e) = validate_sessdata(&cookie_map) {
                    auth_failed = true;
                    checks.push(DoctorCheck {
                        name: check_name,
                        status: CheckStatus::Fail,
                        detail: format!("{e}"),
                        hint: Some("Re-authenticate with `fav auth login` or `fav auth usecookies --stdin`.".into()),
                    });
                    continue;
                }

                let has_csrf = extract_csrf(&cookie_map).is_some();
                let has_refresh = acct
                    .refresh_token
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty());

                let mut notes = vec![format!("state={}", acct.state)];
                if !has_csrf {
                    notes.push("missing bili_jct (like/logout unavailable)".into());
                }
                if has_refresh {
                    notes.push("refresh_token=yes".into());
                }

                if offline {
                    let status = if has_csrf {
                        CheckStatus::Ok
                    } else {
                        CheckStatus::Warn
                    };
                    checks.push(DoctorCheck {
                        name: check_name,
                        status,
                        detail: format!("offline cookie check passed ({})", notes.join(", ")),
                        hint: (!has_csrf).then(|| {
                            "Import full browser cookies with `bili_jct` to enable `like` and remote `logout`."
                                .into()
                        }),
                    });
                } else {
                    match ctx.client_for_account(&acct.cookies) {
                        Ok(client) => match client.nav_info().await {
                            Ok(info) if info.is_login && info.mid == Some(acct.account_id) => {
                                let status = if has_csrf {
                                    CheckStatus::Ok
                                } else {
                                    CheckStatus::Warn
                                };
                                checks.push(DoctorCheck {
                                    name: check_name,
                                    status,
                                    detail: format!("authenticated ({})", notes.join(", ")),
                                    hint: (!has_csrf).then(|| {
                                        "Import full browser cookies with `bili_jct` to enable `like` and remote `logout`."
                                            .into()
                                    }),
                                });
                            }
                            Ok(_) => {
                                auth_failed = true;
                                checks.push(DoctorCheck {
                                    name: check_name,
                                    status: CheckStatus::Fail,
                                    detail: "cookies expired or logged out on Bilibili".into(),
                                    hint: Some(
                                        "Run `fav auth check --refresh` or `fav auth login`."
                                            .into(),
                                    ),
                                });
                            }
                            Err(e) => {
                                non_auth_failed = true;
                                checks.push(DoctorCheck {
                                    name: check_name,
                                    status: CheckStatus::Fail,
                                    detail: format!("network/API error: {e:#}"),
                                    hint: Some(
                                        "Check network connectivity or run `fav doctor --offline`."
                                            .into(),
                                    ),
                                });
                            }
                        },
                        Err(e) => {
                            non_auth_failed = true;
                            checks.push(DoctorCheck {
                                name: check_name,
                                status: CheckStatus::Fail,
                                detail: format!("failed to build HTTP client: {e:#}"),
                                hint: None,
                            });
                        }
                    }
                }
            }
        }
        Err(e) => {
            non_auth_failed = true;
            checks.push(DoctorCheck {
                name: "accounts".into(),
                status: CheckStatus::Fail,
                detail: format!("failed to query accounts: {e:#}"),
                hint: None,
            });
        }
    }

    let table_opts = TableOptions::detect(true, ctx.runtime.color);
    let rendered = match format {
        OutputFormat::Table => render_table(
            ["check", "status", "detail", "hint"],
            checks.iter().map(|c| {
                [
                    c.name.clone(),
                    c.status.badge(table_opts.colored),
                    c.detail.clone(),
                    c.hint.clone().unwrap_or_default(),
                ]
            }),
            table_opts,
        ),
        OutputFormat::Json => render_json(&checks)?,
        OutputFormat::Jsonl => render_jsonl(&checks)?,
        OutputFormat::Tsv => render_tsv(
            ["check", "status", "detail", "hint"],
            checks.iter().map(|c| {
                [
                    c.name.clone(),
                    c.status.as_str().to_owned(),
                    c.detail.clone(),
                    c.hint.clone().unwrap_or_default(),
                ]
            }),
        ),
        OutputFormat::Ids => render_ids(
            checks
                .iter()
                .filter(|c| c.status == CheckStatus::Fail)
                .map(|c| c.name.as_str()),
        ),
    };
    if !rendered.is_empty() || matches!(format, OutputFormat::Json | OutputFormat::Table) {
        println!("{rendered}");
    }

    if non_auth_failed {
        Err(AppError::runtime(anyhow!("Doctor found failing checks.")))
    } else if auth_failed {
        Err(AppError::auth(anyhow!(
            "Doctor found expired or invalid account credentials."
        )))
    } else {
        Ok(())
    }
}
