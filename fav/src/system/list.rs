use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::error;

use crate::{
    db::Db,
    entity::ToTableRecord,
    event::{ListAccount, ListSet},
    runtime::Runtime,
    table::table,
};

pub fn list(mut cmds: Commands) {
    cmds.add_observer(
        |_: Trigger<ListAccount>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let accounts = db.all_accounts().await?;
                let table = table(
                    ["account_id", "name", "state"],
                    accounts.into_iter().map(ToTableRecord::to_record),
                );
                println!("{}\nrows: {}", table, table.count_rows() - 1);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            };
        },
    );
    cmds.add_observer(|_: Trigger<ListSet>, runtime: Res<Runtime>, db: Res<Db>| {
        if let Err(e) = runtime.block_on(async {
            let sets = db.all_sets().await?;
            let table = table(
                ["set_id", "name", "state"],
                sets.into_iter().map(ToTableRecord::to_record),
            );
            println!("{}\nrows: {}", table, table.count_rows() - 1);
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e);
        };
    });
}
