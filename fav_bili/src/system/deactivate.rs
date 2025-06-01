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
        DeactivateAccount, DeactivateAccountAll, DeactivateSet, DeactivateSetAll, DeactivateUp,
        DeactivateUpAll,
    },
    runtime::Runtime,
    state::{AccountState, SetState, UpState},
};

pub fn deactivate(mut cmds: Commands) {
    cmds.add_observer(
        |trigger: Trigger<DeactivateAccount>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let DeactivateAccount { account_id } = *trigger;
                db.deactivate_account(account_id).await?;
                info!("Deactivated account<{}>", account_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<DeactivateAccountAll>,
         mut cmds: Commands,
         runtime: Res<Runtime>,
         db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let accounts = db
                    .get_accounts_filtered(account::Column::State.eq(AccountState::Active))
                    .await?;
                accounts.into_iter().for_each(|account| {
                    cmds.trigger(DeactivateAccount {
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
        |trigger: Trigger<DeactivateSet>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let DeactivateSet { set_id } = *trigger;
                db.deactivate_set(set_id).await?;
                info!("DeactivateSet set<{}>", set_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<DeactivateSetAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let sets = db
                    .get_sets_filtered(set::Column::State.eq(SetState::Active))
                    .await?;
                sets.into_iter()
                    .for_each(|set| cmds.trigger(DeactivateSet { set_id: set.set_id }));
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |trigger: Trigger<DeactivateUp>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let DeactivateUp { up_id } = *trigger;
                db.deactivate_up(up_id).await?;
                info!("Deactivated up<{}>", up_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<DeactivateUpAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let ups = db
                    .get_ups_filtered(up::Column::State.eq(UpState::Active))
                    .await?;
                ups.into_iter()
                    .for_each(|up| cmds.trigger(DeactivateUp { up_id: up.up_id }));
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
}
