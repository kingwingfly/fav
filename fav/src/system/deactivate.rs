use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::{error, info};

use crate::{
    db::Db,
    event::{
        DeactivateAccount, DeactivateAccountAll, DeactivateSet, DeactivateSetAll, DeactivateUp,
        DeactivateUpAll,
    },
    runtime::Runtime,
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
                let accounts = db.all_accounts().await?;
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
                let sets = db.all_sets().await?;
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
                let ups = db.all_ups().await?;
                ups.into_iter()
                    .for_each(|up| cmds.trigger(DeactivateUp { up_id: up.up_id }));
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
}
