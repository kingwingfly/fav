use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use sea_orm::ColumnTrait as _;
use tracing::{error, info};

use crate::{
    db::Db,
    entity::{account, set, up},
    event::{
        ActivateAccount, ActivateAccountAll, ActivateSet, ActivateSetAll, ActivateUp, ActivateUpAll,
    },
    runtime::Runtime,
    state::{AccountState, SetState, UpState},
};

pub fn activate(mut cmds: Commands) {
    cmds.add_observer(
        |trigger: Trigger<ActivateAccount>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let ActivateAccount { account_id } = *trigger;
                db.activate_account(account_id).await?;
                info!("Activated account<{}>", account_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<ActivateAccountAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let accounts = db
                    .get_accounts_filtered(account::Column::State.eq(AccountState::Inactive))
                    .await?;
                accounts.into_iter().for_each(|account| {
                    cmds.trigger(ActivateAccount {
                        account_id: account.account_id,
                    })
                });
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |trigger: Trigger<ActivateSet>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let ActivateSet { set_id } = *trigger;
                db.activate_set(set_id).await?;
                info!("Activated set<{}>", set_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<ActivateSetAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let sets = db
                    .get_sets_filtered(set::Column::State.eq(SetState::Inactive))
                    .await?;
                sets.into_iter()
                    .for_each(|set| cmds.trigger(ActivateSet { set_id: set.set_id }));
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |trigger: Trigger<ActivateUp>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let ActivateUp { up_id } = *trigger;
                db.activate_up(up_id).await?;
                info!("Activated set<{}>", up_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<ActivateUpAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let ups = db
                    .get_ups_filtered(up::Column::State.eq(UpState::Inactive))
                    .await?;
                ups.into_iter()
                    .for_each(|up| cmds.trigger(ActivateUp { up_id: up.up_id }));
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
}
