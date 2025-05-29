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
                let Deactivate { user_id } = *trigger;
                db.deactivate(user_id).await?;
                info!("Deactivated user_id<{}>", user_id);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            }
        },
    );
    cmds.add_observer(
        |_: Trigger<DeactivateAll>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let users = db.all_users().await?;
                users.into_iter().for_each(|user| {
                    cmds.trigger(Deactivate {
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
