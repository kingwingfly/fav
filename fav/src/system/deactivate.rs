use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::{error, info};

use crate::{
    db::Db,
    event::{Deactivate, DeactivateAll},
    runtime::Runtime,
};

pub fn deactivate(mut cmds: Commands) {
    cmds.add_observer(
        |trigger: Trigger<Deactivate>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let Deactivate { account_id } = *trigger;
                db.deactivate(account_id).await?;
                info!("Deactivated account_id<{}>", account_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<DeactivateAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let accounts = db.all_accounts().await?;
                accounts.into_iter().for_each(|account| {
                    cmds.trigger(Deactivate {
                        account_id: account.account_id,
                    })
                });
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
}
