use anyhow::anyhow;
use tracing::{info, warn};

use crate::{
    action::fetch::{FetchOptions, fetch},
    context::AppContext,
    error::{AppError, AppResult},
    picker::{PickerError, PickerItem, pick_multi, resolve_targets_or_fuzzy_match},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Account,
    Set,
    Up,
    All,
}

#[derive(Debug, Clone, Default)]
pub struct ActivateOptions {
    pub targets: Vec<String>,
    pub all: bool,
    pub fetch: bool,
}

fn map_picker_err(err: PickerError, kind_plural: &str) -> AppResult<Vec<i64>> {
    match err {
        PickerError::NonInteractive { .. } => Err(AppError::usage(anyhow!("{err}"))),
        PickerError::Cancelled => {
            info!("Selection cancelled.");
            Ok(Vec::new())
        }
        PickerError::Empty => Err(AppError::usage(anyhow!(
            "No {kind_plural} found in workspace. Run `fav fetch` first."
        ))),
        PickerError::Io(msg) => Err(AppError::runtime(anyhow!("{msg}"))),
    }
}

pub async fn activate(ctx: &AppContext, kind: TargetKind, opts: ActivateOptions) -> AppResult<()> {
    if opts.all && !opts.targets.is_empty() {
        return Err(AppError::usage(anyhow!(
            "--all cannot be combined with explicit targets"
        )));
    }

    match kind {
        TargetKind::All => {
            let accounts = ctx.db.activate_all_accounts().await?;
            let sets = ctx.db.activate_all_sets().await?;
            let ups = ctx.db.activate_all_ups().await?;
            info!("Activated {accounts} account(s), {sets} set(s), and {ups} UP(s).");
            if opts.fetch {
                fetch(ctx, FetchOptions::default()).await?;
            } else if sets + ups > 0 {
                ctx.hint("Run `fav fetch` (or `fav sync`) to index and pull medias for activated sources.");
            }
            Ok(())
        }
        TargetKind::Account => {
            if opts.all {
                let n = ctx.db.activate_all_accounts().await?;
                if n == 0 {
                    warn!("No accounts found. Run `fav auth login` first.");
                } else {
                    info!("Activated {n} account(s).");
                }
            } else {
                let accounts = ctx.db.all_accounts().await?;
                let items: Vec<PickerItem> = accounts
                    .iter()
                    .map(|a| PickerItem {
                        id: a.account_id,
                        label: a.name.clone(),
                        meta: a.state.clone(),
                        selected: false,
                    })
                    .collect();
                let ids = if opts.targets.is_empty() {
                    match pick_multi(
                        "Activate accounts",
                        &items,
                        ctx.runtime.non_interactive,
                        "pass account IDs/names or `--all` (e.g. `fav activate account -a`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "accounts")?,
                    }
                } else {
                    resolve_targets_or_fuzzy_match(&opts.targets, &items)
                        .map_err(AppError::usage)?
                };
                if ids.is_empty() {
                    return Ok(());
                }
                ctx.db
                    .activate_accounts(&ids)
                    .await
                    .map_err(AppError::usage)?;
                info!("Activated {} account(s): {:?}", ids.len(), ids);
            }
            if opts.fetch {
                fetch(ctx, FetchOptions::default()).await?;
            }
            Ok(())
        }
        TargetKind::Set => {
            let activated_ids: Option<Vec<i64>> = if opts.all {
                let n = ctx.db.activate_all_sets().await?;
                if n == 0 {
                    warn!("No sets found. Run `fav fetch` first.");
                } else {
                    info!("Activated {n} set(s).");
                }
                None
            } else {
                let sets = ctx.db.all_sets().await?;
                let items: Vec<PickerItem> = sets
                    .iter()
                    .map(|s| PickerItem {
                        id: s.set_id,
                        label: s.name.clone(),
                        meta: format!("{} medias, {}", s.count, s.state),
                        selected: false,
                    })
                    .collect();
                let ids = if opts.targets.is_empty() {
                    match pick_multi(
                        "Activate favorite sets",
                        &items,
                        ctx.runtime.non_interactive,
                        "pass set IDs/names or `--all` (e.g. `fav activate set <ID>`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "sets")?,
                    }
                } else {
                    resolve_targets_or_fuzzy_match(&opts.targets, &items)
                        .map_err(AppError::usage)?
                };
                if ids.is_empty() {
                    return Ok(());
                }
                ctx.db.activate_sets(&ids).await.map_err(AppError::usage)?;
                info!("Activated {} set(s): {:?}", ids.len(), ids);
                Some(ids)
            };

            if opts.fetch {
                fetch(
                    ctx,
                    FetchOptions {
                        sets_only: true,
                        set_ids: activated_ids.unwrap_or_default(),
                        ..Default::default()
                    },
                )
                .await?;
            } else {
                ctx.hint("Run `fav fetch` (or `fav sync`) to index medias in the newly activated set(s).");
            }
            Ok(())
        }
        TargetKind::Up => {
            let activated_ids: Option<Vec<i64>> = if opts.all {
                let n = ctx.db.activate_all_ups().await?;
                if n == 0 {
                    warn!("No UPs found. Run `fav fetch` first.");
                } else {
                    info!("Activated {n} UP(s).");
                }
                None
            } else {
                let ups = ctx.db.all_ups().await?;
                let items: Vec<PickerItem> = ups
                    .iter()
                    .map(|u| PickerItem {
                        id: u.up_id,
                        label: u.name.clone(),
                        meta: u.state.clone(),
                        selected: false,
                    })
                    .collect();
                let ids = if opts.targets.is_empty() {
                    match pick_multi(
                        "Activate UPs",
                        &items,
                        ctx.runtime.non_interactive,
                        "pass UP IDs/names or `--all` (e.g. `fav activate up <ID>`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "UPs")?,
                    }
                } else {
                    resolve_targets_or_fuzzy_match(&opts.targets, &items)
                        .map_err(AppError::usage)?
                };
                if ids.is_empty() {
                    return Ok(());
                }
                ctx.db.activate_ups(&ids).await.map_err(AppError::usage)?;
                info!("Activated {} UP(s): {:?}", ids.len(), ids);
                Some(ids)
            };

            if opts.fetch {
                fetch(
                    ctx,
                    FetchOptions {
                        ups_only: true,
                        up_ids: activated_ids.unwrap_or_default(),
                        ..Default::default()
                    },
                )
                .await?;
            } else {
                ctx.hint("Run `fav fetch` (or `fav sync`) to index videos for the newly activated UP(s).");
            }
            Ok(())
        }
    }
}
