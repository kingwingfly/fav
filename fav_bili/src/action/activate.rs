use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use tracing::{info, warn};

use crate::{
    action::fetch::{FetchOptions, fetch},
    context::AppContext,
    entity::{account, set, up},
    error::{AppError, AppResult},
    picker::{PickerError, PickerItem, pick_multi, resolve_targets_or_fuzzy_match},
    state::{AccountState, SetState, UpState},
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

fn account_to_picker_item(a: &account::Model) -> PickerItem {
    PickerItem {
        id: a.account_id,
        label: a.name.clone(),
        meta: a.state.clone(),
        selected: false,
    }
}

fn eligible_account_picker_items(accounts: &[account::Model]) -> Vec<PickerItem> {
    accounts
        .iter()
        .filter(|a| a.state != AccountState::Expired.as_str())
        .map(account_to_picker_item)
        .collect()
}

fn set_to_picker_item(s: &set::Model) -> PickerItem {
    PickerItem {
        id: s.set_id,
        label: s.name.clone(),
        meta: format!("{} medias, {}, {}", s.count, s.kind, s.state),
        selected: false,
    }
}

fn eligible_set_picker_items(sets: &[set::Model], active_owned: &HashSet<i64>) -> Vec<PickerItem> {
    sets.iter()
        .filter(|s| active_owned.contains(&s.set_id) && s.state != SetState::Unreachable.as_str())
        .map(set_to_picker_item)
        .collect()
}

fn up_to_picker_item(u: &up::Model) -> PickerItem {
    PickerItem {
        id: u.up_id,
        label: u.name.clone(),
        meta: u.state.clone(),
        selected: false,
    }
}

fn eligible_up_picker_items(ups: &[up::Model], active_followed: &HashSet<i64>) -> Vec<PickerItem> {
    ups.iter()
        .filter(|u| active_followed.contains(&u.up_id) && u.state != UpState::Deactivated.as_str())
        .map(up_to_picker_item)
        .collect()
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
                    if ctx.db.all_accounts().await?.is_empty() {
                        warn!("No accounts found. Run `fav auth login` first.");
                    } else {
                        info!("No inactive accounts to activate (already active or expired).");
                    }
                } else {
                    info!("Activated {n} account(s).");
                }
            } else {
                let accounts = ctx.db.all_accounts().await?;
                let ids = if opts.targets.is_empty() {
                    let picker_items = eligible_account_picker_items(&accounts);
                    if !accounts.is_empty() && picker_items.is_empty() {
                        return Err(AppError::usage(anyhow!(
                            "No non-expired accounts are available to activate (refresh credentials with `fav auth login` or `fav auth check --refresh`, or pass explicit account IDs)."
                        )));
                    }
                    match pick_multi(
                        "Activate accounts",
                        &picker_items,
                        ctx.runtime.non_interactive,
                        "pass account IDs/names or `--all` (e.g. `fav activate account -a`)",
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
                let state_by_id: HashMap<i64, &str> = accounts
                    .iter()
                    .map(|a| (a.account_id, a.state.as_str()))
                    .collect();
                ctx.db
                    .activate_accounts(&ids)
                    .await
                    .map_err(AppError::usage)?;
                info!("Activated {} account(s): {:?}", ids.len(), ids);
                for &account_id in &ids {
                    if state_by_id.get(&account_id).copied() == Some(AccountState::Expired.as_str())
                    {
                        warn!(
                            "Account<{account_id}> was marked Expired; refresh its cookies with `fav auth login` or `fav auth check --refresh` if API calls fail."
                        );
                    }
                }
            }
            if opts.fetch {
                fetch(ctx, FetchOptions::default()).await?;
            }
            Ok(())
        }
        TargetKind::Set => {
            let (activated_ids, activated_count): (Option<Vec<i64>>, u64) = if opts.all {
                let n = ctx.db.activate_all_sets().await?;
                if n == 0 {
                    if ctx.db.all_sets().await?.is_empty() {
                        warn!("No sets found. Run `fav fetch` first.");
                    } else {
                        info!(
                            "No eligible inactive sets to activate (already active, unreachable, or owned only by inactive accounts)."
                        );
                    }
                } else {
                    info!("Activated {n} set(s).");
                }
                (None, n)
            } else {
                let sets = ctx.db.all_sets().await?;
                let active_owned = ctx.db.active_account_set_ids().await?;
                let ids = if opts.targets.is_empty() {
                    let picker_items = eligible_set_picker_items(&sets, &active_owned);
                    if !sets.is_empty() && picker_items.is_empty() {
                        return Err(AppError::usage(anyhow!(
                            "No eligible sets owned by active accounts are available to activate (activate an account with `fav activate account` or pass explicit set IDs)."
                        )));
                    }
                    match pick_multi(
                        "Activate favorite sets",
                        &picker_items,
                        ctx.runtime.non_interactive,
                        "pass set IDs/names or `--all` (e.g. `fav activate set <ID>`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "sets")?,
                    }
                } else {
                    let items: Vec<PickerItem> = sets.iter().map(set_to_picker_item).collect();
                    resolve_targets_or_fuzzy_match(&opts.targets, &items)
                        .map_err(AppError::usage)?
                };
                if ids.is_empty() {
                    return Ok(());
                }
                let state_by_id: HashMap<i64, &str> =
                    sets.iter().map(|s| (s.set_id, s.state.as_str())).collect();
                ctx.db.activate_sets(&ids).await.map_err(AppError::usage)?;
                info!("Activated {} set(s): {:?}", ids.len(), ids);
                for &set_id in &ids {
                    if state_by_id.get(&set_id).copied() == Some(SetState::Unreachable.as_str()) {
                        warn!(
                            "Set<{set_id}> was marked Unreachable; activating it anyway, though fetching may fail if it remains unavailable on Bilibili."
                        );
                    }
                    if !active_owned.contains(&set_id) {
                        warn!(
                            "Set<{set_id}> is not owned by any active account; it will not be synced until an owning account is activated."
                        );
                    }
                }
                let count = ids.len() as u64;
                (Some(ids), count)
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
            } else if activated_count > 0 {
                ctx.hint("Run `fav fetch` (or `fav sync`) to index medias in the newly activated set(s).");
            }
            Ok(())
        }
        TargetKind::Up => {
            let (activated_ids, activated_count): (Option<Vec<i64>>, u64) = if opts.all {
                let n = ctx.db.activate_all_ups().await?;
                if n == 0 {
                    if ctx.db.all_ups().await?.is_empty() {
                        warn!("No UPs found. Run `fav fetch` first.");
                    } else {
                        info!(
                            "No eligible inactive UPs to activate (already active, deactivated, or not followed by any active account)."
                        );
                    }
                } else {
                    info!("Activated {n} UP(s).");
                }
                (None, n)
            } else {
                let ups = ctx.db.all_ups().await?;
                let followed = ctx.db.active_account_up_ids().await?;
                let ids = if opts.targets.is_empty() {
                    let picker_items = eligible_up_picker_items(&ups, &followed);
                    if !ups.is_empty() && picker_items.is_empty() {
                        return Err(AppError::usage(anyhow!(
                            "No eligible followed UPs for active accounts are available to activate (activate an account with `fav activate account`, run `fav fetch`, or pass explicit UP IDs)."
                        )));
                    }
                    match pick_multi(
                        "Activate UPs",
                        &picker_items,
                        ctx.runtime.non_interactive,
                        "pass UP IDs/names or `--all` (e.g. `fav activate up <ID>`)",
                    ) {
                        Ok(ids) => ids,
                        Err(e) => map_picker_err(e, "UPs")?,
                    }
                } else {
                    let items: Vec<PickerItem> = ups.iter().map(up_to_picker_item).collect();
                    resolve_targets_or_fuzzy_match(&opts.targets, &items)
                        .map_err(AppError::usage)?
                };
                if ids.is_empty() {
                    return Ok(());
                }
                let state_by_id: HashMap<i64, &str> =
                    ups.iter().map(|u| (u.up_id, u.state.as_str())).collect();
                ctx.db.activate_ups(&ids).await.map_err(AppError::usage)?;
                info!("Activated {} UP(s): {:?}", ids.len(), ids);
                for &up_id in &ids {
                    if state_by_id.get(&up_id).copied() == Some(UpState::Deactivated.as_str()) {
                        warn!(
                            "UP<{up_id}> was marked Deactivated; activating it anyway, though fetching may fail if the account remains deactivated on Bilibili."
                        );
                    }
                    if !followed.contains(&up_id) {
                        warn!(
                            "UP<{up_id}> is not followed by any active account; it will not be synced until an active account follows it."
                        );
                    }
                }
                let count = ids.len() as u64;
                (Some(ids), count)
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
            } else if activated_count > 0 {
                ctx.hint("Run `fav fetch` (or `fav sync`) to index videos for the newly activated UP(s).");
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
        entity::{set_account, up_account},
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
    fn picker_candidate_helpers_filter_ineligible_items() {
        let accounts = vec![
            mk_account(1, AccountState::Active),
            mk_account(2, AccountState::Inactive),
            mk_account(3, AccountState::Expired),
        ];
        let acc_ids: Vec<i64> = eligible_account_picker_items(&accounts)
            .into_iter()
            .map(|it| it.id)
            .collect();
        assert_eq!(acc_ids, vec![1, 2]);

        let sets = vec![
            mk_set(10, "OwnedInactive", SetState::Inactive),
            mk_set(11, "OwnedActive", SetState::Active),
            mk_set(12, "OwnedUnreachable", SetState::Unreachable),
            mk_set(13, "InactiveAcctSet", SetState::Inactive),
        ];
        let active_owned = HashSet::from([10, 11, 12]);
        let set_ids: Vec<i64> = eligible_set_picker_items(&sets, &active_owned)
            .into_iter()
            .map(|it| it.id)
            .collect();
        assert_eq!(set_ids, vec![10, 11]);

        let ups = vec![
            mk_up(100, "FollowedInactive", UpState::Inactive),
            mk_up(101, "FollowedActive", UpState::Active),
            mk_up(102, "FollowedDeactivated", UpState::Deactivated),
            mk_up(103, "UnfollowedCreator", UpState::Inactive),
        ];
        let active_followed = HashSet::from([100, 101, 102]);
        let up_ids: Vec<i64> = eligible_up_picker_items(&ups, &active_followed)
            .into_iter()
            .map(|it| it.id)
            .collect();
        assert_eq!(up_ids, vec![100, 101]);
    }

    #[tokio::test]
    async fn interactive_picker_errors_when_items_exist_but_none_are_eligible() {
        let (_tmp, ctx) = test_ctx().await;
        ctx.db
            .upsert_account(mk_account(1, AccountState::Active))
            .await
            .unwrap();
        ctx.db
            .upsert_account(mk_account(2, AccountState::Inactive))
            .await
            .unwrap();

        // Only Unreachable (for active acct) and Inactive (for inactive acct) sets exist.
        ctx.db
            .upsert_sets([
                mk_set(10, "UnreachableSet", SetState::Unreachable),
                mk_set(11, "InactiveAcctSet", SetState::Inactive),
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_set_accounts([
                set_account::Model {
                    set_id: 10,
                    account_id: 1,
                },
                set_account::Model {
                    set_id: 11,
                    account_id: 2,
                },
            ])
            .await
            .unwrap();

        // Only Deactivated (for active acct) and unfollowed UPs exist.
        ctx.db
            .upsert_ups([
                mk_up(100, "DeactivatedUp", UpState::Deactivated),
                mk_up(101, "UnfollowedUp", UpState::Inactive),
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_up_accounts([up_account::Model {
                up_id: 100,
                account_id: 1,
            }])
            .await
            .unwrap();

        let set_err = activate(&ctx, TargetKind::Set, ActivateOptions::default())
            .await
            .unwrap_err();
        assert_eq!(set_err.kind(), ExitKind::Usage);
        assert!(
            set_err.to_string().contains("active accounts"),
            "unexpected error: {set_err}"
        );

        let up_err = activate(&ctx, TargetKind::Up, ActivateOptions::default())
            .await
            .unwrap_err();
        assert_eq!(up_err.kind(), ExitKind::Usage);
        assert!(
            up_err.to_string().contains("followed UPs"),
            "unexpected error: {up_err}"
        );

        // Explicit targets still resolve against all_sets() / all_ups() and activate them.
        activate(
            &ctx,
            TargetKind::Set,
            ActivateOptions {
                targets: vec!["10".to_owned(), "11".to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_set(10).await.unwrap().state,
            SetState::Active.as_str()
        );
        assert_eq!(
            ctx.db.get_set(11).await.unwrap().state,
            SetState::Active.as_str()
        );

        activate(
            &ctx,
            TargetKind::Up,
            ActivateOptions {
                targets: vec!["100".to_owned(), "101".to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.db.get_up(100).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(
            ctx.db.get_up(101).await.unwrap().state,
            UpState::Active.as_str()
        );
    }

    #[tokio::test]
    async fn activate_all_target_kind_activates_accounts_before_sets_and_ups_safely() {
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
                mk_set(10, "SetOfInactiveAcct", SetState::Inactive),
                mk_set(11, "UnreachableSet", SetState::Unreachable),
                mk_set(12, "SetOfExpiredAcct", SetState::Inactive),
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_set_accounts([
                set_account::Model {
                    set_id: 10,
                    account_id: 1,
                },
                set_account::Model {
                    set_id: 11,
                    account_id: 1,
                },
                set_account::Model {
                    set_id: 12,
                    account_id: 2,
                },
            ])
            .await
            .unwrap();

        ctx.db
            .upsert_ups([
                mk_up(100, "UpOfInactiveAcct", UpState::Inactive),
                mk_up(101, "DeactivatedUp", UpState::Deactivated),
                mk_up(102, "UpOfExpiredAcct", UpState::Inactive),
                mk_up(103, "UnfollowedUp", UpState::Inactive),
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_up_accounts([
                up_account::Model {
                    up_id: 100,
                    account_id: 1,
                },
                up_account::Model {
                    up_id: 101,
                    account_id: 1,
                },
                up_account::Model {
                    up_id: 102,
                    account_id: 2,
                },
            ])
            .await
            .unwrap();

        activate(&ctx, TargetKind::All, ActivateOptions::default())
            .await
            .unwrap();

        assert_eq!(
            ctx.db.get_account(1).await.unwrap().state,
            AccountState::Active.as_str()
        );
        assert_eq!(
            ctx.db.get_account(2).await.unwrap().state,
            AccountState::Expired.as_str()
        );

        assert_eq!(
            ctx.db.get_set(10).await.unwrap().state,
            SetState::Active.as_str()
        );
        assert_eq!(
            ctx.db.get_set(11).await.unwrap().state,
            SetState::Unreachable.as_str()
        );
        assert_eq!(
            ctx.db.get_set(12).await.unwrap().state,
            SetState::Inactive.as_str()
        );

        assert_eq!(
            ctx.db.get_up(100).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(
            ctx.db.get_up(101).await.unwrap().state,
            UpState::Deactivated.as_str()
        );
        assert_eq!(
            ctx.db.get_up(102).await.unwrap().state,
            UpState::Inactive.as_str()
        );
        assert_eq!(
            ctx.db.get_up(103).await.unwrap().state,
            UpState::Inactive.as_str()
        );
    }
}
