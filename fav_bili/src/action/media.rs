use std::collections::BTreeSet;
use std::path::Path;

use anyhow::anyhow;
use tracing::info;

use crate::{
    context::AppContext,
    db::{MediaFilter, MediaResetFilter},
    error::{AppError, AppResult},
    id_parse::{MediaIdent, parse_media_idents},
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
    pub all: bool,
    pub dry_run: bool,
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

async fn find_missing_media_ids(ctx: &AppContext) -> AppResult<Vec<i64>> {
    let pages = ctx.db.all_completed_pages().await?;
    let output_dir = ctx.effective_output_dir(None);
    let mut missing_ids = BTreeSet::new();
    for page in pages {
        let exists = page.local_path.as_deref().is_some_and(|raw| {
            let p = Path::new(raw);
            if p.is_absolute() {
                p.is_file()
            } else {
                output_dir.join(p).is_file()
            }
        });
        if !exists {
            missing_ids.insert(page.media_id);
        }
    }
    Ok(missing_ids.into_iter().collect())
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

    let mut id_set: BTreeSet<i64> = resolve_media_ids(ctx, &opts.targets)
        .await?
        .into_iter()
        .collect();
    if opts.missing {
        id_set.extend(find_missing_media_ids(ctx).await?);
    }
    let ids: Vec<i64> = id_set.into_iter().collect();

    if opts.dry_run {
        let mut preview_ids = BTreeSet::new();
        for &id in &ids {
            if ctx.db.get_media(id).await.is_ok() {
                preview_ids.insert(id);
            }
        }
        if opts.all {
            for m in ctx.db.all_medias().await? {
                preview_ids.insert(m.id);
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
                    preview_ids.insert(m.id);
                }
            }
        }
        eprintln!(
            "Dry run: would reset {} media(s) to Pending.",
            preview_ids.len()
        );
        return Ok(());
    }

    let mut page_reset_ids: BTreeSet<i64> = ids.iter().copied().collect();
    if opts.all || opts.completed {
        for page in ctx.db.all_completed_pages().await? {
            page_reset_ids.insert(page.media_id);
        }
    }

    let filter = MediaResetFilter {
        ids,
        failed: opts.failed,
        expired: opts.expired,
        permission_denied: opts.permission_denied,
        skipped: opts.skipped,
        completed: opts.completed,
        all: opts.all,
    };
    let count = ctx.db.reset_media_states(&filter).await?;
    for media_id in page_reset_ids {
        ctx.db.reset_pages_for_media(media_id).await?;
    }

    info!("Reset {count} media(s) to Pending.");
    if count > 0 {
        ctx.hint("Run `fav pull` to download pending medias.");
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
