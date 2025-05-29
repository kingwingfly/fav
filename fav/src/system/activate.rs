use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::{error, info};

use crate::{
    db::Db,
    event::{ActivateAccount, ActivateAccountAll, ActivateSet, ActivateSetAll},
    runtime::Runtime,
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
                let accounts = db.all_accounts().await?;
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
                let sets = db.all_sets().await?;
                sets.into_iter()
                    .for_each(|set| cmds.trigger(ActivateSet { set_id: set.set_id }));
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
}
