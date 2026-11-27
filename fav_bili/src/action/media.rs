use std::{
    collections::{BTreeSet, HashSet},
    io::Write as _,
    path::PathBuf,
};

use anyhow::{Context as _, anyhow};
use console::Term;
use tracing::info;

use crate::{
    context::AppContext,
    db::{MediaFilter, MediaResetFilter},
    error::{AppError, AppResult},
    id_parse::{MediaIdent, parse_media_idents},
    reconcile::{
        ReconcileReport, apply_reconciliation, reconcile_completed_medias,
        reconcile_completed_medias_scoped,
    },
    state::MediaState,
};

#[derive(Debug, Clone, Default)]
pub struct MediaResetOptions {
    pub targets: Vec<String>,
    pub failed: bool,
    pub expired: bool,
    pub permission_denied: bool,
    pub skipped: bool,
    pub completed: bool,
    pub missing: bool,
    pub include_inactive: bool,
    pub all: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub output_dir: Option<PathBuf>,
}

async fn ensure_media_exists(ctx: &AppContext, aid: i64, raw: &str) -> AppResult<i64> {
    match ctx.db.get_media(aid).await {
        Ok(model) => Ok(model.id),
        Err(e) if e.downcast_ref::<sea_orm::DbErr>().is_some() => Err(AppError::from(e)),
        Err(_) => Err(AppError::usage(anyhow!("Unknown media '{raw}'"))),
    }
}

async fn resolve_media_ids(ctx: &AppContext, targets: &[String]) -> AppResult<Vec<i64>> {
    let idents = parse_media_idents(targets).map_err(AppError::usage)?;
    let mut ids = Vec::with_capacity(idents.len());
    for (raw, ident) in targets.iter().zip(idents) {
        let id = match ident {
            MediaIdent::Aid(aid) => ensure_media_exists(ctx, aid, raw).await?,
            MediaIdent::Bvid(ref bvid) => {
                if let Some(model) = ctx.db.get_media_by_bvid(bvid).await? {
                    model.id
                } else if let Some(aid) = ident.to_aid() {
                    ensure_media_exists(ctx, aid, raw).await?
                } else {
                    return Err(AppError::usage(anyhow!("Unknown media '{raw}'")));
                }
            }
        };
        ids.push(id);
    }
    Ok(ids)
}

struct ResetPlan {
    explicit_ids: Vec<i64>,
    missing_report: Option<ReconcileReport>,
    inactive_missing_medias: usize,
}

impl ResetPlan {
    fn unconditional_explicit_ids(&self, missing: bool) -> &[i64] {
        if missing { &[] } else { &self.explicit_ids }
    }
}

async fn plan_reset(ctx: &AppContext, opts: &MediaResetOptions) -> AppResult<ResetPlan> {
    let explicit_ids: Vec<i64> = resolve_media_ids(ctx, &opts.targets)
        .await?
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let (missing_report, inactive_missing_medias) = if opts.missing {
        let output_dir = ctx.effective_output_dir(opts.output_dir.as_deref());
        if !explicit_ids.is_empty() {
            let explicit_id_set: HashSet<i64> = explicit_ids.iter().copied().collect();
            let active_only = !opts.include_inactive;
            let rep = reconcile_completed_medias(
                &ctx.db,
                ctx.muxer.as_ref(),
                &output_dir,
                active_only,
                Some(&explicit_id_set),
            )
            .await?;
            let inactive_missing = if active_only {
                let active_ids = ctx.db.active_media_ids().await?;
                let inactive_targets: HashSet<i64> = explicit_id_set
                    .iter()
                    .copied()
                    .filter(|id| !active_ids.contains(id))
                    .collect();
                if inactive_targets.is_empty() {
                    0
                } else {
                    reconcile_completed_medias(
                        &ctx.db,
                        ctx.muxer.as_ref(),
                        &output_dir,
                        false,
                        Some(&inactive_targets),
                    )
                    .await?
                    .affected_missing_media_ids()
                    .len()
                }
            } else {
                0
            };
            (Some(rep), inactive_missing)
        } else if opts.include_inactive {
            let rep =
                reconcile_completed_medias(&ctx.db, ctx.muxer.as_ref(), &output_dir, false, None)
                    .await?;
            (Some(rep), 0usize)
        } else {
            let scoped =
                reconcile_completed_medias_scoped(&ctx.db, ctx.muxer.as_ref(), &output_dir).await?;
            let inactive_missing = scoped.inactive.affected_missing_media_ids().len();
            (Some(scoped.active), inactive_missing)
        }
    } else {
        (None, 0usize)
    };

    Ok(ResetPlan {
        explicit_ids,
        missing_report,
        inactive_missing_medias,
    })
}

async fn compute_dry_run_preview_ids(
    ctx: &AppContext,
    opts: &MediaResetOptions,
    plan: &ResetPlan,
) -> AppResult<BTreeSet<i64>> {
    let mut preview_ids =
        collect_non_missing_selector_ids(ctx, opts, plan.unconditional_explicit_ids(opts.missing))
            .await?;
    if let Some(ref rep) = plan.missing_report {
        preview_ids.extend(rep.affected_missing_media_ids());
    }
    Ok(preview_ids)
}

async fn collect_non_missing_selector_ids(
    ctx: &AppContext,
    opts: &MediaResetOptions,
    explicit_ids: &[i64],
) -> AppResult<BTreeSet<i64>> {
    let mut ids = BTreeSet::new();
    for &id in explicit_ids {
        if ctx.db.get_media(id).await.is_ok() {
            ids.insert(id);
        }
    }
    if opts.all {
        for m in ctx.db.all_medias().await? {
            ids.insert(m.id);
        }
    } else {
        let mut states = Vec::new();
        if opts.failed {
            states.push(MediaState::Failed);
        }
        if opts.expired {
            states.push(MediaState::Expired);
        }
        if opts.permission_denied {
            states.push(MediaState::PermissionDenied);
        }
        if opts.skipped {
            states.push(MediaState::Skipped);
        }
        if opts.completed {
            states.push(MediaState::Completed);
        }
        if !states.is_empty() {
            for m in ctx
                .db
                .query_medias(&MediaFilter {
                    states,
                    ..Default::default()
                })
                .await?
            {
                ids.insert(m.id);
            }
        }
    }
    Ok(ids)
}

async fn build_post_reset_hint(
    ctx: &AppContext,
    reset_ids: &[i64],
) -> AppResult<(usize, Option<String>)> {
    if reset_ids.is_empty() {
        return Ok((0, None));
    }
    let active_reset = ctx.db.count_active_media_ids(reset_ids).await?;
    let inactive_reset = reset_ids.len().saturating_sub(active_reset);
    let active_pending = ctx.db.count_active_pending_medias().await?;

    let hint = if active_pending > 0 {
        if inactive_reset > 0 {
            Some(format!(
                "Run `fav pull` to download {active_pending} active pending media(s) ({inactive_reset} reset media(s) belong to inactive sets/UPs; activate them with `fav activate set` / `fav activate up` first)."
            ))
        } else {
            Some("Run `fav pull` to download pending medias.".to_owned())
        }
    } else {
        Some(format!(
            "{inactive_reset} reset media(s) belong to inactive sets/UPs; activate their set/UP with `fav activate set` / `fav activate up` before pulling."
        ))
    };

    Ok((inactive_reset, hint))
}

pub async fn reset(ctx: &AppContext, opts: MediaResetOptions) -> AppResult<()> {
    let any_selector = !opts.targets.is_empty()
        || opts.failed
        || opts.expired
        || opts.permission_denied
        || opts.skipped
        || opts.completed
        || opts.missing
        || opts.all;

    if !any_selector {
        return Err(AppError::usage(anyhow!(
            "Specify media IDs/BVIDs/URLs or at least one selector (--failed, --expired, --permission-denied, --skipped, --completed, --missing, --all)"
        )));
    }

    let plan = plan_reset(ctx, &opts).await?;

    if opts.dry_run {
        let preview_ids = compute_dry_run_preview_ids(ctx, &opts, &plan).await?;
        if let Some(ref rep) = plan.missing_report {
            eprintln!(
                "Dry run: found {} relocated file(s) (would relink), {} missing page(s) and {} missing legacy media(s); would reset {} media(s) to Pending.",
                rep.relocated.len(),
                rep.missing_pages.len(),
                rep.missing_legacy_ids.len(),
                preview_ids.len()
            );
            if plan.inactive_missing_medias > 0 {
                eprintln!(
                    "Note: {} missing media(s) in inactive sets/UPs skipped (activate their set/UP first or pass --include-inactive).",
                    plan.inactive_missing_medias
                );
            }
        } else {
            eprintln!(
                "Dry run: would reset {} media(s) to Pending.",
                preview_ids.len()
            );
        }
        return Ok(());
    }

    let unconditional_explicit_ids = plan.unconditional_explicit_ids(opts.missing);
    let has_other_selector = !unconditional_explicit_ids.is_empty()
        || opts.failed
        || opts.expired
        || opts.permission_denied
        || opts.skipped
        || opts.completed
        || opts.all;

    let other_reset_ids = if has_other_selector {
        collect_non_missing_selector_ids(ctx, &opts, unconditional_explicit_ids).await?
    } else {
        BTreeSet::new()
    };

    let missing_affected_ids = plan
        .missing_report
        .as_ref()
        .map(ReconcileReport::affected_missing_media_ids)
        .unwrap_or_default();

    let all_completed_page_media_ids: BTreeSet<i64> = if opts.all {
        ctx.db
            .all_completed_pages()
            .await?
            .into_iter()
            .map(|page| page.media_id)
            .collect()
    } else {
        BTreeSet::new()
    };

    let completed_media_ids: BTreeSet<i64> = if !opts.all && opts.completed {
        ctx.db
            .query_medias(&MediaFilter {
                states: vec![MediaState::Completed],
                ..Default::default()
            })
            .await?
            .into_iter()
            .map(|m| m.id)
            .collect()
    } else {
        BTreeSet::new()
    };

    if (opts.all || opts.completed) && !opts.dry_run && !opts.yes {
        let mut confirm_ids: BTreeSet<i64> = other_reset_ids
            .union(&missing_affected_ids)
            .copied()
            .collect();
        confirm_ids.extend(&all_completed_page_media_ids);
        let affected_count = confirm_ids.len();
        if affected_count > 0 {
            let term = Term::stderr();
            if ctx.runtime.non_interactive || !term.is_term() {
                return Err(AppError::usage(anyhow!(
                    "Pass --yes to reset --all / --completed medias non-interactively (or preview with --dry-run)."
                )));
            }
            eprint!(
                "Reset {affected_count} media(s) (including Completed tracking) back to Pending? [y/N] "
            );
            let _ = std::io::stderr().flush();
            let answer = term
                .read_line()
                .context("Failed reading confirmation from terminal")?;
            let normalized = answer.trim().to_ascii_lowercase();
            if normalized != "y" && normalized != "yes" {
                eprintln!("Aborted.");
                return Ok(());
            }
        }
    }

    let mut missing_reset_ids = BTreeSet::new();

    // 1. Handle `--missing` surgically: relink renamed/moved files and reset ONLY missing pages,
    // preserving intact pages of multi-part videos!
    if let Some(ref rep) = plan.missing_report {
        missing_reset_ids = missing_affected_ids;
        let applied = apply_reconciliation(&ctx.db, rep).await?;
        if applied.relinked_pages > 0 {
            info!(
                "Relinked {} relocated/renamed file(s) in DB.",
                applied.relinked_pages
            );
        }
        if !opts.include_inactive && plan.inactive_missing_medias > 0 {
            if applied.reset_medias == 0 {
                info!(
                    "0 active missing media(s) found ({} missing in inactive sets/UPs skipped; activate their set/UP first or pass --include-inactive).",
                    plan.inactive_missing_medias
                );
            } else {
                info!(
                    "Skipped {} missing media(s) in inactive sets/UPs (activate their set/UP first or pass --include-inactive).",
                    plan.inactive_missing_medias
                );
            }
        }
    }

    // 2. Handle explicit targets (when not scoping `--missing`) and state selectors
    // (deduplicating IDs already reset by `--missing`).
    let remaining_explicit_ids: Vec<i64> = unconditional_explicit_ids
        .iter()
        .copied()
        .filter(|id| !missing_reset_ids.contains(id))
        .collect();

    if has_other_selector {
        let mut page_reset_ids: BTreeSet<i64> =
            unconditional_explicit_ids.iter().copied().collect();
        if opts.all {
            page_reset_ids.extend(&other_reset_ids);
            page_reset_ids.extend(&all_completed_page_media_ids);
        } else if opts.completed {
            page_reset_ids.extend(&completed_media_ids);
        }

        let filter = MediaResetFilter {
            ids: remaining_explicit_ids,
            failed: opts.failed,
            expired: opts.expired,
            permission_denied: opts.permission_denied,
            skipped: opts.skipped,
            completed: opts.completed,
            all: opts.all,
        };
        ctx.db.reset_media_states(&filter).await?;
        for media_id in page_reset_ids {
            ctx.db.reset_pages_for_media(media_id).await?;
        }
    }

    let mut all_reset_ids = missing_reset_ids;
    all_reset_ids.extend(other_reset_ids);
    let reset_ids_vec: Vec<i64> = all_reset_ids.into_iter().collect();

    info!("Reset {} media(s) to Pending.", reset_ids_vec.len());
    let (inactive_reset, hint) = build_post_reset_hint(ctx, &reset_ids_vec).await?;
    if inactive_reset > 0 {
        info!(
            "{inactive_reset} reset media(s) belong to inactive sets/UPs and will not be pulled until activated."
        );
    }
    if let Some(msg) = hint {
        ctx.hint(&msg);
    }
    Ok(())
}

pub async fn skip(ctx: &AppContext, targets: Vec<String>) -> AppResult<()> {
    if targets.is_empty() {
        return Err(AppError::usage(anyhow!(
            "Specify one or more media IDs, BVIDs, or URLs to skip."
        )));
    }
    let ids = resolve_media_ids(ctx, &targets).await?;
    let count = ctx.db.skip_medias(&ids).await?;
    info!("Marked {count} media(s) as Skipped.");
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
        downloader::file_size_and_inode,
        entity::{account, media, media_set, set, set_account},
        muxer::test_support::MockMuxer,
        state::{AccountState, MediaPageState, MediaType, SetKind, SetState},
    };

    async fn seed_active_and_inactive_sets(db: &Db) {
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
                set_id: 100,
                name: "ActiveSet".to_owned(),
                count: 3,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 200,
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
    }

    #[tokio::test]
    async fn reset_missing_respects_output_dir_relinks_renamed_and_surgically_resets_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let custom_out = tmp.path().join("custom_out");
        std::fs::create_dir_all(&custom_out).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_active_and_inactive_sets(&db).await;
        db.upsert_medias([
            media::Model {
                id: 10,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "MultiPart".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 20,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "Renamed".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 30,
                bv_id: "BV1xx411c7mD".to_owned(),
                title: "LegacyMissing".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 40,
                bv_id: "BV1inactive040".to_owned(),
                title: "InactiveMissing".to_owned(),
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
                set_id: 100,
            },
            media_set::Model {
                id: 30,
                set_id: 100,
            },
            media_set::Model {
                id: 40,
                set_id: 200,
            },
        ])
        .await
        .unwrap();

        // Media 10: P1 exists in custom_out, P2 missing
        let p1 = custom_out.join("10-MultiPart(1)-P1.mp4");
        std::fs::write(&p1, b"p1-bytes").unwrap();
        let (sz1, ino1) = file_size_and_inode(&p1);
        db.mark_page_completed(
            10,
            101,
            1,
            "P1".to_owned(),
            "10-MultiPart(1)-P1.mp4".to_owned(),
            sz1,
            ino1,
        )
        .await
        .unwrap();
        db.mark_page_completed(
            10,
            102,
            2,
            "P2".to_owned(),
            "10-MultiPart(2)-P2.mp4".to_owned(),
            Some(99),
            None,
        )
        .await
        .unwrap();

        // Media 20: renamed with BVID in filename
        let m20_renamed = custom_out.join("renamed_BV1Q541167Qg.mp4");
        std::fs::write(&m20_renamed, b"m20-bytes").unwrap();
        db.mark_page_completed(
            20,
            201,
            1,
            "P1".to_owned(),
            "20-Renamed.mp4".to_owned(),
            Some(9),
            None,
        )
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );

        // Dry run must NOT mutate DB
        reset(
            &ctx,
            MediaResetOptions {
                missing: true,
                dry_run: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(10).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(20).await.unwrap()[0]
                .local_path
                .as_deref(),
            Some("20-Renamed.mp4")
        );

        // Real reset --missing with `-o custom_out` (active-only by default!)
        reset(
            &ctx,
            MediaResetOptions {
                missing: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Media 10: reset to Pending, P1 stays Completed, ONLY P2 becomes Pending!
        assert_eq!(
            ctx.db.get_media(10).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        let pages_10 = ctx.db.pages_for_media(10).await.unwrap();
        assert_eq!(pages_10[0].state, MediaPageState::Completed.as_str());
        assert_eq!(pages_10[1].state, MediaPageState::Pending.as_str());

        // Media 20: relinked and remains Completed!
        assert_eq!(
            ctx.db.get_media(20).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(20).await.unwrap()[0]
                .local_path
                .as_deref(),
            Some("renamed_BV1Q541167Qg.mp4")
        );

        // Media 30 (active legacy v1.x): reset to Pending!
        assert_eq!(
            ctx.db.get_media(30).await.unwrap().state,
            MediaState::Pending.as_str()
        );

        // Media 40 (inactive legacy v1.x): SKIPPED by default, remains Completed!
        assert_eq!(
            ctx.db.get_media(40).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // Explicit `--include-inactive` resets inactive media 40 too!
        reset(
            &ctx,
            MediaResetOptions {
                missing: true,
                include_inactive: true,
                output_dir: Some(custom_out),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(40).await.unwrap().state,
            MediaState::Pending.as_str()
        );
    }

    #[tokio::test]
    async fn post_reset_hint_never_suggests_fav_pull_when_no_active_pending_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let custom_out = tmp.path().join("out");
        std::fs::create_dir_all(&custom_out).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_active_and_inactive_sets(&db).await;
        db.upsert_medias([
            media::Model {
                id: 50,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "InactiveMissing".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 60,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "ActiveCompleted".to_owned(),
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
                id: 50,
                set_id: 200,
            },
            media_set::Model {
                id: 60,
                set_id: 100,
            },
        ])
        .await
        .unwrap();
        std::fs::write(custom_out.join("60-ActiveCompleted.mp4"), b"ok").unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );

        // Default `--missing` finds 0 active missing -> media 50 stays Completed!
        reset(
            &ctx,
            MediaResetOptions {
                missing: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(50).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        let (inactive_cnt, none_hint) = build_post_reset_hint(&ctx, &[]).await.unwrap();
        assert_eq!(inactive_cnt, 0);
        assert!(none_hint.is_none());

        // Resetting inactive media 50 with `--include-inactive`: 0 active pending -> hint tells user to activate, NEVER `Run fav pull`!
        reset(
            &ctx,
            MediaResetOptions {
                missing: true,
                include_inactive: true,
                output_dir: Some(custom_out),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(50).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        let (inactive_cnt, inactive_hint) = build_post_reset_hint(&ctx, &[50]).await.unwrap();
        assert_eq!(inactive_cnt, 1);
        let msg = inactive_hint.expect("expected inactive activation hint");
        assert!(
            !msg.contains("fav pull"),
            "inactive-only reset must not suggest fav pull: {msg}"
        );
        assert!(msg.contains("fav activate set"));

        // Resetting active media 60 -> active pending > 0 -> suggests `fav pull`!
        ctx.db
            .set_media_state(60, MediaState::Pending)
            .await
            .unwrap();
        let (inactive_cnt, active_hint) = build_post_reset_hint(&ctx, &[60]).await.unwrap();
        assert_eq!(inactive_cnt, 0);
        assert_eq!(
            active_hint.as_deref(),
            Some("Run `fav pull` to download pending medias.")
        );

        // Resetting mixed [50, 60] -> hints active count + notes inactive reset!
        let (inactive_cnt, mixed_hint) = build_post_reset_hint(&ctx, &[50, 60]).await.unwrap();
        assert_eq!(inactive_cnt, 1);
        let mixed_msg = mixed_hint.expect("expected mixed hint");
        assert!(mixed_msg.contains("Run `fav pull` to download 1 active pending media(s)"));
        assert!(mixed_msg.contains("1 reset media(s) belong to inactive sets/UPs"));
    }

    #[tokio::test]
    async fn reset_all_and_completed_confirmation_guard_and_exemptions() {
        use crate::error::ExitKind;

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_active_and_inactive_sets(&db).await;
        db.upsert_medias([
            media::Model {
                id: 1,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "CompletedOne".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 2,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "FailedOne".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Failed.to_string(),
                retry_count: 2,
                last_error: Some("err".to_owned()),
                updated_at: None,
            },
            media::Model {
                id: 3,
                bv_id: "BV1xx411c7mD".to_owned(),
                title: "SkippedOne".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Skipped.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 1, set_id: 100 },
            media_set::Model { id: 2, set_id: 100 },
            media_set::Model { id: 3, set_id: 100 },
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

        // Non-interactive `--all` without `--yes` fails with Usage and mutates nothing
        let err = reset(
            &ctx,
            MediaResetOptions {
                all: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(err.to_string().contains("Pass --yes"));
        assert_eq!(
            ctx.db.get_media(1).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // Non-interactive `--completed` without `--yes` fails with Usage and mutates nothing
        let err = reset(
            &ctx,
            MediaResetOptions {
                completed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(err.to_string().contains("Pass --yes"));
        assert_eq!(
            ctx.db.get_media(1).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // `--dry-run` works without `--yes` for both `--all` and `--completed`
        reset(
            &ctx,
            MediaResetOptions {
                all: true,
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        reset(
            &ctx,
            MediaResetOptions {
                completed: true,
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(1).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // Non-destructive selectors (`--failed`, `--skipped`, explicit IDs) never prompt
        reset(
            &ctx,
            MediaResetOptions {
                failed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(2).await.unwrap().state,
            MediaState::Pending.as_str()
        );

        reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["3".to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(3).await.unwrap().state,
            MediaState::Pending.as_str()
        );

        // `--completed --yes` succeeds and resets Completed media to Pending
        reset(
            &ctx,
            MediaResetOptions {
                completed: true,
                yes: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(1).await.unwrap().state,
            MediaState::Pending.as_str()
        );

        // `--completed` without `--yes` when 0 Completed medias exist is a clean no-op
        reset(
            &ctx,
            MediaResetOptions {
                completed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // `--all --yes` succeeds and resets all medias
        ctx.db
            .set_media_state(1, MediaState::Completed)
            .await
            .unwrap();
        reset(
            &ctx,
            MediaResetOptions {
                all: true,
                yes: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(1).await.unwrap().state,
            MediaState::Pending.as_str()
        );
    }

    #[tokio::test]
    async fn reset_completed_preserves_completed_pages_of_failed_multipage_media() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();
        seed_active_and_inactive_sets(&db).await;

        // 0 Completed medias, 1 Failed multi-page media whose P1 already completed.
        db.upsert_medias([media::Model {
            id: 70,
            bv_id: "BV1failmp00070".to_owned(),
            title: "PartiallyFailedMultiPage".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Failed.to_string(),
            retry_count: 1,
            last_error: Some("part 2 failed".to_owned()),
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([media_set::Model {
            id: 70,
            set_id: 100,
        }])
        .await
        .unwrap();
        db.mark_page_completed(
            70,
            7001,
            1,
            "P1".to_owned(),
            "70-PartiallyFailedMultiPage(1)-P1.mp4".to_owned(),
            Some(1234),
            None,
        )
        .await
        .unwrap();
        db.mark_page_failed(70, 7002, 2, "P2".to_owned())
            .await
            .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );

        // `media reset --completed` with 0 Completed medias must leave the Failed media's P1 intact!
        reset(
            &ctx,
            MediaResetOptions {
                completed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            ctx.db.get_media(70).await.unwrap().state,
            MediaState::Failed.as_str()
        );
        let pages_after_noop = ctx.db.pages_for_media(70).await.unwrap();
        assert_eq!(
            pages_after_noop[0].state,
            MediaPageState::Completed.as_str()
        );
        assert_eq!(pages_after_noop[1].state, MediaPageState::Failed.as_str());

        // Even when a Completed media DOES exist, `--completed --yes` resets ONLY the Completed
        // media's pages and leaves the Failed media's completed page intact.
        ctx.db
            .upsert_medias([media::Model {
                id: 71,
                bv_id: "BV1comp0000071".to_owned(),
                title: "CompletedMultiPage".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_media_sets([media_set::Model {
                id: 71,
                set_id: 100,
            }])
            .await
            .unwrap();
        ctx.db
            .mark_page_completed(
                71,
                7101,
                1,
                "P1".to_owned(),
                "71-CompletedMultiPage(1)-P1.mp4".to_owned(),
                Some(2048),
                None,
            )
            .await
            .unwrap();

        reset(
            &ctx,
            MediaResetOptions {
                completed: true,
                yes: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            ctx.db.get_media(71).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(71).await.unwrap()[0].state,
            MediaPageState::Pending.as_str()
        );
        assert_eq!(
            ctx.db.get_media(70).await.unwrap().state,
            MediaState::Failed.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(70).await.unwrap()[0].state,
            MediaPageState::Completed.as_str()
        );

        // `--all --yes` resets both the Failed media and its completed page.
        reset(
            &ctx,
            MediaResetOptions {
                all: true,
                yes: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(70).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(70).await.unwrap()[0].state,
            MediaPageState::Pending.as_str()
        );
    }

    #[tokio::test]
    async fn reset_and_skip_reject_unknown_targets_with_usage_error() {
        use crate::error::ExitKind;

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();
        seed_active_and_inactive_sets(&db).await;
        db.upsert_medias([media::Model {
            id: 10,
            bv_id: "BV17x411w7KC".to_owned(),
            title: "Tracked".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([media_set::Model {
            id: 10,
            set_id: 100,
        }])
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );

        let reset_err = reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["999999999".to_owned()],
                ..Default::default()
            },
        )
        .await
        .expect_err("unknown AID on reset must error");
        assert_eq!(reset_err.kind(), ExitKind::Usage);
        assert!(reset_err.to_string().contains("Unknown media"));

        let reset_bv_err = reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["BV1Q541167Qg".to_owned()],
                ..Default::default()
            },
        )
        .await
        .expect_err("unknown BVID on reset must error");
        assert_eq!(reset_bv_err.kind(), ExitKind::Usage);

        let skip_err = skip(&ctx, vec!["999999999".to_owned()])
            .await
            .expect_err("unknown AID on skip must error");
        assert_eq!(skip_err.kind(), ExitKind::Usage);
        assert!(skip_err.to_string().contains("Unknown media"));

        // Mixed valid + unknown target on skip fails atomically without mutating media 10.
        let mixed_skip_err = skip(&ctx, vec!["10".to_owned(), "999999999".to_owned()])
            .await
            .expect_err("mixed known/unknown target on skip must error");
        assert_eq!(mixed_skip_err.kind(), ExitKind::Usage);
        assert_eq!(
            ctx.db.get_media(10).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // Known target succeeds on skip.
        skip(&ctx, vec!["10".to_owned()]).await.unwrap();
        assert_eq!(
            ctx.db.get_media(10).await.unwrap().state,
            MediaState::Skipped.as_str()
        );
    }

    #[tokio::test]
    async fn reset_target_scoped_missing_respects_disk_presence_dry_run_and_isolates_unrelated() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let custom_out = tmp.path().join("out");
        std::fs::create_dir_all(&custom_out).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_active_and_inactive_sets(&db).await;
        db.upsert_medias([
            media::Model {
                id: 10,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "PresentTarget".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 20,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "PartialMissingTarget".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 30,
                bv_id: "BV1xx411c7mD".to_owned(),
                title: "UnrelatedActiveMissing".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 40,
                bv_id: "BV1inactive040".to_owned(),
                title: "InactiveMissing".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 50,
                bv_id: "BV1failed00050".to_owned(),
                title: "FailedMedia".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Failed.to_string(),
                retry_count: 2,
                last_error: Some("transient".to_owned()),
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
                set_id: 100,
            },
            media_set::Model {
                id: 30,
                set_id: 100,
            },
            media_set::Model {
                id: 40,
                set_id: 200,
            },
            media_set::Model {
                id: 50,
                set_id: 100,
            },
        ])
        .await
        .unwrap();

        // Media 10: present on disk!
        let m10_p1 = custom_out.join("10-PresentTarget.mp4");
        std::fs::write(&m10_p1, b"present-bytes").unwrap();
        let (sz10, ino10) = file_size_and_inode(&m10_p1);
        db.mark_page_completed(
            10,
            1001,
            1,
            "P1".to_owned(),
            "10-PresentTarget.mp4".to_owned(),
            sz10,
            ino10,
        )
        .await
        .unwrap();

        // Media 20: multi-page where P1 is present and P2 is missing on disk.
        let m20_p1 = custom_out.join("20-PartialMissingTarget(1)-P1.mp4");
        std::fs::write(&m20_p1, b"m20-p1-bytes").unwrap();
        let (sz20, ino20) = file_size_and_inode(&m20_p1);
        db.mark_page_completed(
            20,
            2001,
            1,
            "P1".to_owned(),
            "20-PartialMissingTarget(1)-P1.mp4".to_owned(),
            sz20,
            ino20,
        )
        .await
        .unwrap();
        db.mark_page_completed(
            20,
            2002,
            2,
            "P2".to_owned(),
            "20-PartialMissingTarget(2)-P2.mp4".to_owned(),
            Some(99),
            None,
        )
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );

        // 1. Present target with `--missing --dry-run` MUST preview 0 resets (NOT 1!).
        let present_dry_opts = MediaResetOptions {
            targets: vec!["10".to_owned()],
            missing: true,
            dry_run: true,
            output_dir: Some(custom_out.clone()),
            ..Default::default()
        };
        let present_plan = plan_reset(&ctx, &present_dry_opts).await.unwrap();
        let present_preview = compute_dry_run_preview_ids(&ctx, &present_dry_opts, &present_plan)
            .await
            .unwrap();
        assert!(
            present_preview.is_empty(),
            "present target with --missing --dry-run must preview 0 resets, got {present_preview:?}"
        );
        reset(&ctx, present_dry_opts).await.unwrap();

        // 2. Present target with `--missing` (real run) MUST NOT reset media 10 or touch 20/30/40.
        reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["10".to_owned()],
                missing: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(10).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(10).await.unwrap()[0].state,
            MediaPageState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.get_media(20).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.get_media(30).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.get_media(40).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // 3. Present target with `--missing --failed` resets failed media 50 without resetting present media 10.
        reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["10".to_owned()],
                missing: true,
                failed: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(10).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.get_media(50).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        assert_eq!(
            ctx.db.get_media(20).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.get_media(30).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // 4. Target 20 with `--missing`: surgically resets ONLY P2 of media 20, leaving P1 Completed
        // and leaving unrelated missing media 30 (active) and 40 (inactive) untouched!
        let missing_dry_opts = MediaResetOptions {
            targets: vec!["20".to_owned()],
            missing: true,
            dry_run: true,
            output_dir: Some(custom_out.clone()),
            ..Default::default()
        };
        let missing_plan = plan_reset(&ctx, &missing_dry_opts).await.unwrap();
        let missing_preview = compute_dry_run_preview_ids(&ctx, &missing_dry_opts, &missing_plan)
            .await
            .unwrap();
        assert_eq!(missing_preview, BTreeSet::from([20]));

        reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["20".to_owned()],
                missing: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(20).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        let pages_20 = ctx.db.pages_for_media(20).await.unwrap();
        assert_eq!(pages_20[0].state, MediaPageState::Completed.as_str());
        assert_eq!(pages_20[1].state, MediaPageState::Pending.as_str());
        assert_eq!(
            ctx.db.get_media(30).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.get_media(40).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        // 5. Inactive target 40 with `--missing`: skipped without `--include-inactive`, reset with it,
        // while still never touching unrelated active missing media 30!
        reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["40".to_owned()],
                missing: true,
                output_dir: Some(custom_out.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(40).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        reset(
            &ctx,
            MediaResetOptions {
                targets: vec!["40".to_owned()],
                missing: true,
                include_inactive: true,
                output_dir: Some(custom_out),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_media(40).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        assert_eq!(
            ctx.db.get_media(30).await.unwrap().state,
            MediaState::Completed.as_str()
        );
    }
}
