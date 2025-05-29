use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::error;

use crate::{db::Db, entity::ToTableRecord, event::ListUser, runtime::Runtime, table::table};

pub fn list(mut cmds: Commands) {
    cmds.add_observer(|_: Trigger<ListUser>, runtime: Res<Runtime>, db: Res<Db>| {
        if let Err(e) = runtime.block_on(async {
            let users = db.all_users().await?;
            println!(
                "{}",
                table(
                    ["user id", "name", "state"],
                    users.into_iter().map(ToTableRecord::to_record)
                )
            );
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e);
        };
    });
}
