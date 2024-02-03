use crate::api::{fetch::fetch, pull::pull_all};
use rand::Rng;
use tokio::time::Duration;
use tracing::info;

pub(crate) async fn interval(interval: u64) {
    if interval <= 15 {
        panic!("Interval would better to be greater than 15 minutes.");
    }
    job().await;
    let mut rng = rand::thread_rng();
    loop {
        let interval = (interval as i64 + rng.gen_range(-5..=5)) as u64;
        info!("Next job will be {} minutes later.", interval);
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
