use anyhow::Result;
use tracing::info;

use crate::db::db;

pub async fn activate_account(account_id: i64) -> Result<()> {
    let db = db().await;
    db.activate_account(account_id).await?;
    info!("Activated account<{}>", account_id);
    Ok(())
}

pub async fn activate_account_all() -> Result<()> {
    let db = db().await;
    db.activate_all_accounts().await?;
    info!("Activated all accounts");
    Ok(())
}

pub async fn activate_set(set_id: i64) -> Result<()> {
    let db = db().await;
    db.activate_set(set_id).await?;
    info!("Activated set<{}>", set_id);
    Ok(())
}

pub async fn activate_set_all() -> Result<()> {
    let db = db().await;
    db.activate_all_sets().await?;
    info!("Activated all sets");
    Ok(())
}

pub async fn activate_up(up_id: i64) -> Result<()> {
    let db = db().await;
    db.activate_up(up_id).await?;
    info!("Activated up<{}>", up_id);
    Ok(())
}

pub async fn activate_up_all() -> Result<()> {
    let db = db().await;
    db.activate_all_ups().await?;
    info!("Activated all ups");
    Ok(())
}
