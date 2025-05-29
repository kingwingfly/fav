use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::{error, info};

use crate::{
    db::Db,
    event::{Activate, ActivateAll},
    runtime::Runtime,
};

pub fn activate(mut cmds: Commands) {
    cmds.add_observer(
        |trigger: Trigger<Activate>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let Activate { account_id } = *trigger;
                db.activate(account_id).await?;
                info!("Activated account_id<{}>", account_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<ActivateAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let accounts = db.all_accounts().await?;
                accounts.into_iter().for_each(|account| {
                    cmds.trigger(Activate {
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
