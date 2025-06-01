use anyhow::Result;
use tracing::info;

use crate::db::db;

pub async fn deactivate_account(account_id: i64) -> Result<()> {
    let db = db().await;
    db.deactivate_account(account_id).await?;
    info!("Deactivated account<{}>", account_id);
    Ok(())
}

pub async fn deactivate_account_all() -> Result<()> {
    let db = db().await;
    db.deactivate_all_accounts().await?;
    info!("Deactivated all accounts");
    Ok(())
}

pub async fn deactivate_set(set_id: i64) -> Result<()> {
    let db = db().await;
    db.deactivate_set(set_id).await?;
    info!("Deactivated set<{}>", set_id);
    Ok(())
}

pub async fn deactivate_set_all() -> Result<()> {
    let db = db().await;
    db.deactivate_all_sets().await?;
    info!("Deactivated all sets");
    Ok(())
}

pub async fn deactivate_up(up_id: i64) -> Result<()> {
    let db = db().await;
    db.deactivate_up(up_id).await?;
    info!("Deactivated up<{}>", up_id);
    Ok(())
}

pub async fn deactivate_up_all() -> Result<()> {
    let db = db().await;
    db.deactivate_all_ups().await?;
    info!("Deactivated all ups");
    Ok(())
}
