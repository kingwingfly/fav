use anyhow::anyhow;
use tracing::{info, warn};

use crate::{
    action::activate::TargetKind,
    context::AppContext,
    error::{AppError, AppResult},
    picker::{PickerError, PickerItem, pick_multi, resolve_targets_or_fuzzy_match},
};

#[derive(Debug, Clone, Default)]
pub struct DeactivateOptions {
    pub targets: Vec<String>,
    pub all: bool,
}

fn map_picker_err(err: PickerError, kind_plural: &str) -> AppResult<Vec<i64>> {
    match err {
        PickerError::NonInteractive { .. } => Err(AppError::usage(anyhow!("{err}"))),
        PickerError::Cancelled => {
            info!("Selection cancelled.");
            Ok(Vec::new())
        }
        PickerError::Empty => Err(AppError::usage(anyhow!(
            "No {kind_plural} found in workspace."
        ))),
        PickerError::Io(msg) => Err(AppError::runtime(anyhow!("{msg}"))),
    }
}

pub async fn deactivate(
    ctx: &AppContext,
    kind: TargetKind,
    opts: DeactivateOptions,
) -> AppResult<()> {
    if opts.all && !opts.targets.is_empty() {
        return Err(AppError::usage(anyhow!(
            "--all cannot be combined with explicit targets"
        )));
    }

    match kind {
        TargetKind::All => {
            let accounts = ctx.db.deactivate_all_accounts().await?;
            let sets = ctx.db.deactivate_all_sets().await?;
            let ups = ctx.db.deactivate_all_ups().await?;
            info!("Deactivated {accounts} account(s), {sets} set(s), and {ups} UP(s).");
            Ok(())
        }
        TargetKind::Account => {
            if opts.all {
                let n = ctx.db.deactivate_all_accounts().await?;
                if n == 0 {
                    warn!("No accounts found.");
                } else {
                    info!("Deactivated {n} account(s).");
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
                        "Deactivate accounts",
                        &items,
                        ctx.runtime.non_interactive,
                        "pass account IDs/names or `--all` (e.g. `fav deactivate account -a`)",
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
                    .deactivate_accounts(&ids)
                    .await
                    .map_err(AppError::usage)?;
                info!("Deactivated {} account(s): {:?}", ids.len(), ids);
            }
            Ok(())
        }
        TargetKind::Set => {
            if opts.all {
                let n = ctx.db.deactivate_all_sets().await?;
                if n == 0 {
                    warn!("No sets found.");
                } else {
                    info!("Deactivated {n} set(s).");
                }
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
                        "Deactivate favorite sets",
                        &items,
                        ctx.runtime.non_interactive,
                        "pass set IDs/names or `--all` (e.g. `fav deactivate set <ID>`)",
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
                ctx.db
                    .deactivate_sets(&ids)
                    .await
                    .map_err(AppError::usage)?;
                info!("Deactivated {} set(s): {:?}", ids.len(), ids);
            }
            Ok(())
        }
        TargetKind::Up => {
            if opts.all {
                let n = ctx.db.deactivate_all_ups().await?;
                if n == 0 {
                    warn!("No UPs found.");
                } else {
                    info!("Deactivated {n} UP(s).");
                }
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
                        "Deactivate UPs",
                        &items,
                        ctx.runtime.non_interactive,
                        "pass UP IDs/names or `--all` (e.g. `fav deactivate up <ID>`)",
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
                ctx.db.deactivate_ups(&ids).await.map_err(AppError::usage)?;
                info!("Deactivated {} UP(s): {:?}", ids.len(), ids);
            }
            Ok(())
        }
    }
}
