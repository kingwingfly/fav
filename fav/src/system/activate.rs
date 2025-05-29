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
                let Activate { user_id } = *trigger;
                db.activate(user_id).await?;
                info!("Activated user_id<{}>", user_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<ActivateAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let users = db.all_users().await?;
                users.into_iter().for_each(|user| {
                    cmds.trigger(Activate {
                        user_id: user.user_id,
                    })
                });
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
}
