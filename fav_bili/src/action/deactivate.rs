use std::collections::HashSet;

use anyhow::anyhow;
use tracing::{info, warn};

use crate::{
    action::activate::TargetKind,
    context::AppContext,
    entity::{account, set, up},
    error::{AppError, AppResult},
    picker::{PickerError, PickerItem, pick_multi, resolve_targets_or_fuzzy_match},
    state::{AccountState, SetState, UpState},
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

fn account_to_picker_item(a: &account::Model) -> PickerItem {
    PickerItem {
        id: a.account_id,
        label: a.name.clone(),
        meta: a.state.clone(),
        selected: false,
    }
}

fn active_account_picker_items(accounts: &[account::Model]) -> Vec<PickerItem> {
    accounts
        .iter()
        .filter(|a| a.state == AccountState::Active.as_str())
        .map(account_to_picker_item)
        .collect()
}

fn set_to_picker_item(s: &set::Model, active_owned: Option<&HashSet<i64>>) -> PickerItem {
    let suffix = match active_owned {
        Some(owned) if !owned.contains(&s.set_id) => ", no active account",
        _ => "",
    };
    PickerItem {
        id: s.set_id,
        label: s.name.clone(),
        meta: format!("{} medias, {}, {}{suffix}", s.count, s.kind, s.state),
        selected: false,
    }
}

fn active_set_picker_items(sets: &[set::Model], active_owned: &HashSet<i64>) -> Vec<PickerItem> {
    sets.iter()
        .filter(|s| s.state == SetState::Active.as_str())
        .map(|s| set_to_picker_item(s, Some(active_owned)))
        .collect()
}

fn up_to_picker_item(u: &up::Model, active_followed: Option<&HashSet<i64>>) -> PickerItem {
    let meta = match active_followed {
        Some(followed) if !followed.contains(&u.up_id) => format!("{}, unfollowed", u.state),
        _ => u.state.clone(),
    };
    PickerItem {
        id: u.up_id,
        label: u.name.clone(),
        meta,
        selected: false,
    }
}

fn active_up_picker_items(ups: &[up::Model], active_followed: &HashSet<i64>) -> Vec<PickerItem> {
    ups.iter()
        .filter(|u| u.state == UpState::Active.as_str())
        .map(|u| up_to_picker_item(u, Some(active_followed)))
        .collect()
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
                    if ctx.db.all_accounts().await?.is_empty() {
                        warn!("No accounts found.");
                    } else {
                        info!("No active accounts to deactivate.");
                    }
                } else {
                    info!("Deactivated {n} account(s).");
                }
            } else {
                let accounts = ctx.db.all_accounts().await?;
                let ids = if opts.targets.is_empty() {
                    let picker_items = active_account_picker_items(&accounts);
                    if !accounts.is_empty() && picker_items.is_empty() {
                        return Err(AppError::usage(anyhow!(
                            "No active accounts to deactivate."
                        )));
                    }
                    match pick_multi(
                        "Deactivate accounts",
                        &picker_items,
                        ctx.runtime.non_interactive,
                        "pass account IDs/names or `--all` (e.g. `fav deactivate account -a`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "accounts")?,
                    }
                } else {
                    let items: Vec<PickerItem> =
                        accounts.iter().map(account_to_picker_item).collect();
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
                    if ctx.db.all_sets().await?.is_empty() {
                        warn!("No sets found.");
                    } else {
                        info!("No active sets to deactivate.");
                    }
                } else {
                    info!("Deactivated {n} set(s).");
                }
            } else {
                let sets = ctx.db.all_sets().await?;
                let ids = if opts.targets.is_empty() {
                    let active_owned = ctx.db.active_account_set_ids().await?;
                    let picker_items = active_set_picker_items(&sets, &active_owned);
                    if !sets.is_empty() && picker_items.is_empty() {
                        return Err(AppError::usage(anyhow!("No active sets to deactivate.")));
                    }
                    match pick_multi(
                        "Deactivate favorite sets",
                        &picker_items,
                        ctx.runtime.non_interactive,
                        "pass set IDs/names or `--all` (e.g. `fav deactivate set <ID>`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "sets")?,
                    }
                } else {
                    let items: Vec<PickerItem> =
                        sets.iter().map(|s| set_to_picker_item(s, None)).collect();
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
                    if ctx.db.all_ups().await?.is_empty() {
                        warn!("No UPs found.");
                    } else {
                        info!("No active UPs to deactivate.");
                    }
                } else {
                    info!("Deactivated {n} UP(s).");
                }
            } else {
                let ups = ctx.db.all_ups().await?;
                let ids = if opts.targets.is_empty() {
                    let followed = ctx.db.active_account_up_ids().await?;
                    let picker_items = active_up_picker_items(&ups, &followed);
                    if !ups.is_empty() && picker_items.is_empty() {
                        return Err(AppError::usage(anyhow!("No active UPs to deactivate.")));
                    }
                    match pick_multi(
                        "Deactivate UPs",
                        &picker_items,
                        ctx.runtime.non_interactive,
                        "pass UP IDs/names or `--all` (e.g. `fav deactivate up <ID>`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "UPs")?,
                    }
                } else {
                    let items: Vec<PickerItem> =
                        ups.iter().map(|u| up_to_picker_item(u, None)).collect();
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        api::BiliClientConfig,
        config::{Config, Workspace},
        db::Db,
        error::ExitKind,
        muxer::test_support::MockMuxer,
        state::SetKind,
    };

    async fn test_ctx() -> (tempfile::TempDir, AppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();
        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );
        (tmp, ctx)
    }

    fn mk_account(id: i64, state: AccountState) -> account::Model {
        account::Model {
            account_id: id,
            name: format!("acct_{id}"),
            cookies: "SESSDATA=x".to_owned(),
            state: state.to_string(),
            refresh_token: None,
        }
    }

    fn mk_set(id: i64, name: &str, state: SetState) -> set::Model {
        set::Model {
            set_id: id,
            name: name.to_owned(),
            count: 5,
            state: state.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: false,
            resume_page: None,
        }
    }

    fn mk_up(id: i64, name: &str, state: UpState) -> up::Model {
        up::Model {
            up_id: id,
            name: name.to_owned(),
            state: state.to_string(),
            sync_complete: false,
            resume_page: None,
        }
    }

    #[test]
    fn deactivate_picker_shows_only_active_items_and_labels_unfollowed_or_unowned() {
        let accounts = vec![
            mk_account(1, AccountState::Active),
            mk_account(2, AccountState::Inactive),
            mk_account(3, AccountState::Expired),
        ];
        let acc_items = active_account_picker_items(&accounts);
        assert_eq!(acc_items.len(), 1);
        assert_eq!(acc_items[0].id, 1);

        let sets = vec![
            mk_set(10, "ActiveOwned", SetState::Active),
            mk_set(11, "ActiveInactiveAcct", SetState::Active),
            mk_set(12, "InactiveSet", SetState::Inactive),
            mk_set(13, "UnreachableSet", SetState::Unreachable),
        ];
        let set_items = active_set_picker_items(&sets, &HashSet::from([10]));
        assert_eq!(set_items.len(), 2);
        assert_eq!(set_items[0].id, 10);
        assert!(!set_items[0].meta.contains("no active account"));
        assert_eq!(set_items[1].id, 11);
        assert!(set_items[1].meta.contains("no active account"));

        // Active unfollowed UP MUST remain selectable in deactivate picker, while Inactive unfollowed
        // creators and Deactivated UPs are hidden.
        let ups = vec![
            mk_up(100, "ActiveFollowed", UpState::Active),
            mk_up(101, "ActiveUnfollowed", UpState::Active),
            mk_up(102, "InactiveUnfollowed", UpState::Inactive),
            mk_up(103, "DeactivatedUp", UpState::Deactivated),
        ];
        let up_items = active_up_picker_items(&ups, &HashSet::from([100]));
        assert_eq!(up_items.len(), 2);
        assert_eq!(up_items[0].id, 100);
        assert_eq!(up_items[0].meta, "Active");
        assert_eq!(up_items[1].id, 101);
        assert_eq!(up_items[1].meta, "Active, unfollowed");
    }

    #[tokio::test]
    async fn deactivate_picker_errors_when_no_active_items_and_all_preserves_terminal_states() {
        let (_tmp, ctx) = test_ctx().await;
        ctx.db
            .upsert_account(mk_account(1, AccountState::Inactive))
            .await
            .unwrap();
        ctx.db
            .upsert_account(mk_account(2, AccountState::Expired))
            .await
            .unwrap();
        ctx.db
            .upsert_sets([
                mk_set(10, "InactiveSet", SetState::Inactive),
                mk_set(11, "UnreachableSet", SetState::Unreachable),
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_ups([
                mk_up(100, "InactiveUp", UpState::Inactive),
                mk_up(101, "DeactivatedUp", UpState::Deactivated),
            ])
            .await
            .unwrap();

        for kind in [TargetKind::Account, TargetKind::Set, TargetKind::Up] {
            let err = deactivate(&ctx, kind, DeactivateOptions::default())
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ExitKind::Usage);
            assert!(err.to_string().contains("No active"));
        }

        // Activate one of each and verify TargetKind::All preserves Expired / Unreachable / Deactivated.
        ctx.db.activate_account(1).await.unwrap();
        ctx.db.activate_set(10).await.unwrap();
        ctx.db.activate_up(100).await.unwrap();

        deactivate(&ctx, TargetKind::All, DeactivateOptions::default())
            .await
            .unwrap();
        assert_eq!(
            ctx.db.get_account(1).await.unwrap().state,
            AccountState::Inactive.as_str()
        );
        assert_eq!(
            ctx.db.get_account(2).await.unwrap().state,
            AccountState::Expired.as_str()
        );
        assert_eq!(
            ctx.db.get_set(10).await.unwrap().state,
            SetState::Inactive.as_str()
        );
        assert_eq!(
            ctx.db.get_set(11).await.unwrap().state,
            SetState::Unreachable.as_str()
        );
        assert_eq!(
            ctx.db.get_up(100).await.unwrap().state,
            UpState::Inactive.as_str()
        );
        assert_eq!(
            ctx.db.get_up(101).await.unwrap().state,
            UpState::Deactivated.as_str()
        );
    }
}
