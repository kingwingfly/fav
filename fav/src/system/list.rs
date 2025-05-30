use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::error;

use crate::{
    db::Db,
    entity::ToTableRecord,
    event::{ListAccount, ListMedia, ListSet, ListUp},
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
                ["set_id", "name", "count", "state"],
                sets.into_iter().map(ToTableRecord::to_record),
            );
            println!("{}\nrows: {}", table, table.count_rows() - 1);
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e);
        };
    });
    cmds.add_observer(
        |_: Trigger<ListMedia>, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let medias = db.all_medias().await?;
                let table = table(
                    ["id", "bvid", "title", "type", "state"],
                    medias.into_iter().map(ToTableRecord::to_record),
                );
                println!("{}\nrows: {}", table, table.count_rows() - 1);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            };
        },
    );
    cmds.add_observer(|_: Trigger<ListUp>, runtime: Res<Runtime>, db: Res<Db>| {
        if let Err(e) = runtime.block_on(async {
            let ups = db.all_ups().await?;
            let table = table(
                ["id", "name"],
                ups.into_iter().map(ToTableRecord::to_record),
            );
            println!("{}\nrows: {}", table, table.count_rows() - 1);
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e);
        };
    });
}
