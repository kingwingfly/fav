use anyhow::Result;

use crate::{db::db, entity::ToTableRecord, table::table};

pub async fn list_accounts() -> Result<()> {
    let db = db(false).await;
    let accounts = db.all_accounts().await?;
    let table = table(
        ["account_id", "name", "state"],
        accounts.into_iter().map(ToTableRecord::to_record),
    );
    println!("{}\nrows: {}", table, table.count_rows() - 1);
    Ok(())
}

pub async fn list_sets() -> Result<()> {
    let db = db(false).await;
    let sets = db.all_sets().await?;
    let table = table(
        ["set_id", "name", "count", "state"],
        sets.into_iter().map(ToTableRecord::to_record),
    );
    println!("{}\nrows: {}", table, table.count_rows() - 1);
    Ok(())
}

pub async fn list_medias() -> Result<()> {
    let db = db(false).await;
    let medias = db.all_medias().await?;
    let table = table(
        ["id", "bvid", "title", "type", "state"],
        medias.into_iter().map(ToTableRecord::to_record),
    );
    println!("{}\nrows: {}", table, table.count_rows() - 1);
    Ok(())
}

pub async fn list_ups() -> Result<()> {
    let db = db(false).await;
    let ups = db.all_ups().await?;
    let table = table(
        ["id", "name", "state"],
        ups.into_iter().map(ToTableRecord::to_record),
    );
    println!("{}\nrows: {}", table, table.count_rows() - 1);
    Ok(())
}
