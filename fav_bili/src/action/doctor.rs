use std::path::{Path, PathBuf};

use anyhow::anyhow;
use console::style;
use serde::Serialize;

use crate::{
    context::AppContext,
    cookies::{extract_csrf, parse_cookie_header, validate_sessdata},
    error::{AppError, AppResult},
    reconcile::reconcile_completed_medias_scoped,
    table::{
        OutputFormat, TableOptions, render_ids, render_json, render_jsonl, render_table,
        render_tsv, write_stdout_line,
    },
};

#[derive(Debug, Clone)]
pub struct DoctorOptions {
    pub format: OutputFormat,
    pub offline: bool,
    pub output_dir: Option<PathBuf>,
}

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

fn unescape_proc_mount_path(raw: &str) -> String {
    raw.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

/// Resolve the longest-matching mount point's filesystem type from `/proc/mounts` contents.
pub(crate) fn find_mount_fstype_in_proc_mounts(path: &Path, proc_mounts: &str) -> Option<String> {
    let mut best_len: usize = 0;
    let mut best_fs: Option<String> = None;

    for line in proc_mounts.lines() {
        let mut fields = line.split_whitespace();
        let _device = fields.next()?;
        let Some(raw_mount) = fields.next() else {
            continue;
        };
        let Some(fs_type) = fields.next() else {
            continue;
        };
        let mount_str = unescape_proc_mount_path(raw_mount);
        let mount_path = Path::new(&mount_str);
        if path.starts_with(mount_path) {
            let len = mount_str.len();
            if len >= best_len {
                best_len = len;
                best_fs = Some(fs_type.to_owned());
            }
        }
    }
    best_fs
}

#[cfg(target_os = "linux")]
fn resolve_for_mount_lookup(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut cur = abs.as_path();
    let mut suffix = Vec::new();
    loop {
        if cur.exists() {
            let mut base = std::fs::canonicalize(cur).unwrap_or_else(|_| cur.to_path_buf());
            for comp in suffix.into_iter().rev() {
                base.push(comp);
            }
            return base;
        }
        match (cur.file_name(), cur.parent()) {
            (Some(name), Some(parent)) => {
                suffix.push(name.to_os_string());
                cur = parent;
            }
            _ => return abs,
        }
    }
}

fn check_staging_fs(tmp_dir: &Path) -> Option<DoctorCheck> {
    #[cfg(target_os = "linux")]
    {
        let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
        let resolved = resolve_for_mount_lookup(tmp_dir);
        let fs_type = find_mount_fstype_in_proc_mounts(&resolved, &mounts)?;
        if matches!(fs_type.as_str(), "tmpfs" | "ramfs") {
            return Some(DoctorCheck {
                name: "staging_fs".into(),
                status: CheckStatus::Warn,
                detail: format!(
                    "staging directory {} resides on in-memory filesystem '{}' (risk of exhausting RAM on large downloads)",
                    tmp_dir.display(),
                    fs_type
                ),
                hint: Some(
                    "Set `tmp_dir` in `.fav/config.toml` to a persistent on-disk directory.".into(),
                ),
            });
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = tmp_dir;
        None
    }
}

pub async fn doctor(ctx: &AppContext, format: OutputFormat, offline: bool) -> AppResult<()> {
    doctor_with_options(
        ctx,
        DoctorOptions {
            format,
            offline,
            output_dir: None,
        },
    )
    .await
}

pub async fn doctor_with_options(ctx: &AppContext, opts: DoctorOptions) -> AppResult<()> {
    let (checks, non_auth_failed, auth_failed) = collect_doctor_checks(ctx, &opts).await;

    let table_opts = TableOptions::detect(true, ctx.runtime.color);
    let rendered = match opts.format {
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
    if !rendered.is_empty() || matches!(opts.format, OutputFormat::Json | OutputFormat::Table) {
        write_stdout_line(&rendered).map_err(AppError::runtime)?;
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

async fn collect_doctor_checks(
    ctx: &AppContext,
    opts: &DoctorOptions,
) -> (Vec<DoctorCheck>, bool, bool) {
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

    // 4. DB-vs-disk drift (4-layer rename-safe reconciliation scoped to active resources, matching `fav pull --recheck` 1-to-1)
    let output_dir = ctx.effective_output_dir(opts.output_dir.as_deref());
    match reconcile_completed_medias_scoped(&ctx.db, ctx.muxer.as_ref(), &output_dir).await {
        Ok(scoped) => {
            let report = scoped.active;
            let inactive_missing = scoped.inactive.total_missing_items();
            let verified = report.total_verified();
            let relocated = report.relocated.len();
            let missing = report.total_missing_items();
            let total = report.total_checked();

            if !report.has_drift() {
                let mut detail = if report.total_checked_legacy > 0 {
                    format!(
                        "{verified} active completed item(s) verified on disk ({} page(s), {} legacy media(s))",
                        report.verified_pages, report.verified_legacy
                    )
                } else {
                    format!("{verified} active completed item(s) verified on disk")
                };
                if inactive_missing > 0 {
                    detail.push_str(&format!(
                        "; {inactive_missing} missing in inactive sets/UPs ignored"
                    ));
                }
                checks.push(DoctorCheck {
                    name: "disk_sync".into(),
                    status: CheckStatus::Ok,
                    detail,
                    hint: None,
                });
            } else {
                let mut parts = vec![format!("{verified} verified")];
                if relocated > 0 {
                    parts.push(format!("{relocated} relocated/renamed"));
                }
                if missing > 0 {
                    if report.missing_legacy_ids.is_empty() {
                        parts.push(format!("{missing} missing page(s)"));
                    } else if report.missing_pages.is_empty() {
                        parts.push(format!(
                            "{} missing legacy media(s)",
                            report.missing_legacy_ids.len()
                        ));
                    } else {
                        parts.push(format!(
                            "{} missing page(s) + {} missing legacy media(s)",
                            report.missing_pages.len(),
                            report.missing_legacy_ids.len()
                        ));
                    }
                }
                let inactive_note = if inactive_missing > 0 {
                    format!("; {inactive_missing} missing in inactive sets/UPs ignored")
                } else {
                    String::new()
                };
                checks.push(DoctorCheck {
                    name: "disk_sync".into(),
                    status: CheckStatus::Warn,
                    detail: format!(
                        "{} (of {total} active under {}){inactive_note}",
                        parts.join(", "),
                        output_dir.display()
                    ),
                    hint: Some(
                        "Run `fav pull --recheck` to relink renamed active files and re-download missing ones, or `fav media reset --missing` to re-queue them."
                            .into(),
                    ),
                });
            }
        }
        Err(e) => {
            non_auth_failed = true;
            checks.push(DoctorCheck {
                name: "disk_sync".into(),
                status: CheckStatus::Fail,
                detail: format!("failed to reconcile completed medias: {e:#}"),
                hint: None,
            });
        }
    }

    // 4b. Staging directory checks (leftover `.part` files + Linux tmpfs/ramfs warning for Issue #153)
    let tmp_dir = ctx.effective_tmp_dir();
    if let Some(fs_check) = check_staging_fs(&tmp_dir) {
        checks.push(fs_check);
    }

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
                hint: Some(format!(
                    "Run `fav pull` to resume partial downloads or delete `{}/*.part`.",
                    tmp_dir.display()
                )),
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

                if opts.offline {
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

    (checks, non_auth_failed, auth_failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_mounts_lookup_picks_longest_matching_mount_and_unescapes_spaces() {
        let sample = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev 0 0
tmpfs /run tmpfs rw,nosuid,nodev 0 0
/dev/sda1 /run/media/user/My\\040Drive btrfs rw,relatime 0 0
ramfs /var/ramdisk ramfs rw 0 0
";
        assert_eq!(
            find_mount_fstype_in_proc_mounts(Path::new("/tmp/fav/.fav/tmp"), sample).as_deref(),
            Some("tmpfs")
        );
        // Must NOT confuse /tmp2 with /tmp
        assert_eq!(
            find_mount_fstype_in_proc_mounts(Path::new("/tmp2/fav"), sample).as_deref(),
            Some("ext4")
        );
        // Longest mount wins over /run tmpfs!
        assert_eq!(
            find_mount_fstype_in_proc_mounts(Path::new("/run/media/user/My Drive/fav"), sample)
                .as_deref(),
            Some("btrfs")
        );
        assert_eq!(
            find_mount_fstype_in_proc_mounts(Path::new("/var/ramdisk/stage"), sample).as_deref(),
            Some("ramfs")
        );
    }

    #[tokio::test]
    async fn doctor_with_options_respects_output_dir_and_warns_without_failing() {
        use std::sync::Arc;

        use crate::{
            api::BiliClientConfig,
            config::{Config, Workspace},
            db::Db,
            entity::{account, media, media_set, set, set_account},
            muxer::test_support::MockMuxer,
            state::{AccountState, MediaState, MediaType, SetKind, SetState},
        };

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let custom_out = tmp.path().join("out");
        std::fs::create_dir_all(&custom_out).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=s; bili_jct=c".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_sets([
            set::Model {
                set_id: 100,
                name: "ActiveSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 200,
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
                set_id: 100,
                account_id: 1,
            },
            set_account::Model {
                set_id: 200,
                account_id: 1,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 10,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "ActiveLegacyMissing".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 20,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "InactiveLegacyMissing".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model {
                id: 10,
                set_id: 100,
            },
            media_set::Model {
                id: 20,
                set_id: 200,
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

        let opts = DoctorOptions {
            format: OutputFormat::Json,
            offline: true,
            output_dir: Some(custom_out.clone()),
        };

        // Active missing + inactive missing -> Warn with active hint + inactive note.
        let (checks, non_auth_failed, auth_failed) = collect_doctor_checks(&ctx, &opts).await;
        assert!(!non_auth_failed && !auth_failed);
        let disk_sync = checks.iter().find(|c| c.name == "disk_sync").unwrap();
        assert_eq!(disk_sync.status, CheckStatus::Warn);
        assert!(disk_sync.detail.contains("1 missing legacy media(s)"));
        assert!(
            disk_sync
                .detail
                .contains("1 missing in inactive sets/UPs ignored")
        );
        assert!(
            disk_sync
                .hint
                .as_deref()
                .unwrap()
                .contains("fav pull --recheck")
        );

        doctor_with_options(&ctx, opts.clone())
            .await
            .expect("doctor with warn-only disk drift should succeed");

        // Place the active file on disk so ONLY the inactive legacy media is missing ->
        // disk_sync MUST be Ok and NEVER suggest `fav pull --recheck`!
        std::fs::write(custom_out.join("10-ActiveLegacyMissing.mp4"), b"ok").unwrap();
        let (checks_ok, _, _) = collect_doctor_checks(&ctx, &opts).await;
        let disk_sync_ok = checks_ok.iter().find(|c| c.name == "disk_sync").unwrap();
        assert_eq!(disk_sync_ok.status, CheckStatus::Ok);
        assert!(
            disk_sync_ok
                .detail
                .contains("1 active completed item(s) verified on disk")
        );
        assert!(
            disk_sync_ok
                .detail
                .contains("1 missing in inactive sets/UPs ignored")
        );
        assert!(
            disk_sync_ok.hint.is_none(),
            "doctor must not suggest fav pull --recheck when only inactive items are missing: {:?}",
            disk_sync_ok.hint
        );
    }
}
