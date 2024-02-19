use crate::api::{fetch::fetch, pull::pull_all};
use tokio::time::Duration;
use tracing::{info, warn};

pub(crate) async fn interval(interval: u64) {
    if interval < 15 {
        warn!("Interval would better to be greater than 15 minutes.");
        return;
    }
    job().await;
    loop {
        let next_ts_local = (chrono::Utc::now() + chrono::Duration::minutes(interval as i64))
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        info!(
            "Next job will be {} minutes later at {}.",
            interval, next_ts_local
        );
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval * 60)) => {
                job().await;
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }
}

async fn job() {
    fetch(false, false).await.ok();
    pull_all().await;
}
