use std::{collections::BTreeSet, path::PathBuf};

use anyhow::anyhow;
use tracing::info;

use crate::{
    context::AppContext,
    db::{MediaFilter, MediaResetFilter},
    error::{AppError, AppResult},
    id_parse::{MediaIdent, parse_media_idents},
    reconcile::{
        apply_reconciliation, reconcile_completed_medias, reconcile_completed_medias_scoped,
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
    pub output_dir: Option<PathBuf>,
}

async fn resolve_media_ids(ctx: &AppContext, targets: &[String]) -> AppResult<Vec<i64>> {
    let idents = parse_media_idents(targets).map_err(AppError::usage)?;
    let mut ids = Vec::with_capacity(idents.len());
    for ident in idents {
        match ident {
            MediaIdent::Aid(aid) => ids.push(aid),
            MediaIdent::Bvid(ref bvid) => {
                if let Some(aid) = ident.to_aid() {
                    ids.push(aid);
                } else if let Some(model) = ctx.db.get_media_by_bvid(bvid).await? {
                    ids.push(model.id);
                } else {
                    return Err(AppError::usage(anyhow!("Unknown media '{bvid}'")));
                }
            }
        }
    }
    Ok(ids)
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

    let explicit_ids: Vec<i64> = resolve_media_ids(ctx, &opts.targets)
        .await?
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let (missing_report, inactive_missing_medias) = if opts.missing {
        let output_dir = ctx.effective_output_dir(opts.output_dir.as_deref());
        if opts.include_inactive {
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

    if opts.dry_run {
        let mut preview_ids = collect_non_missing_selector_ids(ctx, &opts, &explicit_ids).await?;
        if let Some(ref rep) = missing_report {
            preview_ids.extend(rep.affected_missing_media_ids());
        }
        if let Some(ref rep) = missing_report {
            eprintln!(
                "Dry run: found {} relocated file(s) (would relink), {} missing page(s) and {} missing legacy media(s); would reset {} media(s) to Pending.",
                rep.relocated.len(),
                rep.missing_pages.len(),
                rep.missing_legacy_ids.len(),
                preview_ids.len()
            );
            if inactive_missing_medias > 0 {
                eprintln!(
                    "Note: {inactive_missing_medias} missing media(s) in inactive sets/UPs skipped (activate their set/UP first or pass --include-inactive)."
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

    let has_other_selector = !explicit_ids.is_empty()
        || opts.failed
        || opts.expired
        || opts.permission_denied
        || opts.skipped
        || opts.completed
        || opts.all;

    let other_reset_ids = if has_other_selector {
        collect_non_missing_selector_ids(ctx, &opts, &explicit_ids).await?
    } else {
        BTreeSet::new()
    };

    let mut missing_reset_ids = BTreeSet::new();

    // 1. Handle `--missing` surgically: relink renamed/moved files and reset ONLY missing pages,
    // preserving intact pages of multi-part videos!
    if let Some(ref rep) = missing_report {
        missing_reset_ids = rep.affected_missing_media_ids();
        let applied = apply_reconciliation(&ctx.db, rep).await?;
        if applied.relinked_pages > 0 {
            info!(
                "Relinked {} relocated/renamed file(s) in DB.",
                applied.relinked_pages
            );
        }
        if !opts.include_inactive && inactive_missing_medias > 0 {
            if applied.reset_medias == 0 {
                info!(
                    "0 active missing media(s) found ({inactive_missing_medias} missing in inactive sets/UPs skipped; activate their set/UP first or pass --include-inactive)."
                );
            } else {
                info!(
                    "Skipped {inactive_missing_medias} missing media(s) in inactive sets/UPs (activate their set/UP first or pass --include-inactive)."
                );
            }
        }
    }

    // 2. Handle explicit targets and state selectors (deduplicating IDs already reset by `--missing`).
    let remaining_explicit_ids: Vec<i64> = explicit_ids
        .iter()
        .copied()
        .filter(|id| !missing_reset_ids.contains(id))
        .collect();

    if has_other_selector {
        let mut page_reset_ids: BTreeSet<i64> = explicit_ids.iter().copied().collect();
        if opts.all || opts.completed {
            for page in ctx.db.all_completed_pages().await? {
                page_reset_ids.insert(page.media_id);
            }
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
}
