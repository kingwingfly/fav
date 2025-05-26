use fav_core::ops::{ResOpsExt as _, SetOpsExt as _};
use fav_core::prelude::*;
use fav_core::status::SetStatusExt as _;
use fav_core::visual::{TableRes as _, TableSet as _, TableSets as _};
use fav_utils::bili::{Bili, BiliSets};
use fav_utils::FavUtilsError;
use std::io::Write as _;
use tracing::{error, info, warn};

pub(super) fn init() -> FavCoreResult<()> {
    let path = std::path::PathBuf::from(BiliSets::PATH);
    if path.exists() {
        let mut stdout = std::io::stdout();
        stdout
            .write_all(b"The .fav folder already exists, do you want to overwrite it? (y/n): ")?;
        stdout.flush()?;
        let stdin = std::io::stdin();
        let mut buf = String::new();
        stdin.read_line(&mut buf)?;
        if buf.trim() != "y" {
            return Ok(());
        }
    }
    BiliSets::default().write()?;
    info!("The .fav folder has been initialized.");
    Ok(())
}

pub(super) async fn login() -> FavCoreResult<()> {
    let mut bili = Bili::default();
    bili.login().await?;
    bili.write()
}

pub(super) async fn logout() -> FavCoreResult<()> {
    let mut bili = Bili::read()?;
    bili.logout().await
}

pub(super) fn status(sets: &mut BiliSets, id: String) -> FavCoreResult<()> {
    let id = Id::from(&id);
    for set in sets.iter_mut() {
        if set.id() == id {
            set.table();
            return Ok(());
        } else if let Some(res) = set.iter().find(|r| r.id() == id) {
            res.table();
            return Ok(());
        }
    }
    error!(
        "{}",
        FavCoreError::IdNotUsable {
            id: id.to_string(),
            msg: "ID not found".to_string(),
        }
    );
    Ok(())
}

pub(super) fn status_all(
    sets: &mut BiliSets,
    show_sets: bool,
    show_res: bool,
    only_track: bool,
    show_all: bool,
) -> FavCoreResult<()> {
    if show_sets {
        let sub = sets.subset(|s| {
            (s.check_status(StatusFlags::TRACK) | !only_track)
                & ((s.upper.mid == 0 // self-created set's mid is 0
                    && !s.check_status(StatusFlags::EXPIRED))
                    | show_all)
        });
        sub.table();
    }
    if show_res {
        for set in sets.iter_mut() {
            let sub = set.subset(|r| r.check_status(StatusFlags::TRACK) | !only_track);
            sub.table();
        }
    }
    Ok(())
}

pub(super) async fn fetch(sets: &mut BiliSets) -> FavCoreResult<()> {
    let bili = Bili::read()?;
    bili.fetch_sets(sets).await?;
    let mut sub =
        sets.subset(|s| s.check_status(StatusFlags::TRACK) & !s.check_status(StatusFlags::EXPIRED));
    bili.batch_fetch_set(&mut sub, 8).await?;
    for set in sub.iter_mut() {
        let mut sub = set.subset(|r| {
            r.check_status(StatusFlags::TRACK)
                & !r.check_status(StatusFlags::FETCHED)
                & !r.check_status(StatusFlags::EXPIRED)
        });
        bili.batch_fetch_res(&mut sub, 8).await?;
    }
    Ok(())
}

pub(super) fn track(sets: &mut BiliSets, ids: Vec<String>) -> FavCoreResult<()> {
    'a: for id in ids.iter().map(Id::from) {
        for set in sets.iter_mut() {
            if set.id() == id {
                set.on_status(StatusFlags::TRACK);
                set.on_res_status(StatusFlags::TRACK);
                info!("Tracked set ID: {id}");
                continue 'a;
            } else if let Some(res) = set.iter_mut().find(|r| r.id() == id) {
                res.on_status(StatusFlags::TRACK);
                info!("Tracked resource ID: {id}");
                continue 'a;
            }
        }
        error!(
            "{}",
            FavCoreError::IdNotUsable {
                id: id.to_string(),
                msg: "ID not found".to_string(),
            }
        );
    }

    Ok(())
}

pub(super) fn untrack(sets: &mut BiliSets, ids: Vec<String>) -> FavCoreResult<()> {
    'a: for id in ids.iter().map(Id::from) {
        for set in sets.iter_mut() {
            if set.id() == id {
                set.off_status(StatusFlags::TRACK);
                set.off_res_status(StatusFlags::TRACK);
                info!("Untracked set ID: {id}");
                continue 'a;
            } else if let Some(res) = set.iter_mut().find(|r| r.id() == id) {
                res.off_status(StatusFlags::TRACK);
                info!("Untracked resource ID: {id}");
                continue 'a;
            }
        }
        error!(
            "{}",
            FavCoreError::IdNotUsable {
                id: id.to_string(),
                msg: "ID not found".to_string(),
            }
        );
    }

    Ok(())
}

pub(super) async fn pull_all(sets: &mut BiliSets) -> FavCoreResult<()> {
    fetch(sets).await?;
    let bili = Bili::read()?;
    let mut sub = sets.subset(|s| s.check_status(StatusFlags::TRACK));
    for set in sub.iter_mut() {
        let mut sub = set.subset(|r| {
            !r.check_status(StatusFlags::SAVED)
                & !r.check_status(StatusFlags::EXPIRED)
                & r.check_status(StatusFlags::TRACK | StatusFlags::FETCHED)
        });
        bili.batch_pull_res(&mut sub, 8).await?;
    }
    Ok(())
}

pub(super) async fn pull(sets: &mut BiliSets, ids: Vec<String>) -> FavCoreResult<()> {
    fetch(sets).await?;
    let ids: Vec<_> = ids.iter().map(Id::from).collect();
    let bili = Bili::read()?;
    for id in ids.iter() {
        if let Some(s) = sets.iter_mut().find(|s| s.id() == *id) {
            let mut sub = s.subset(|r| {
                !r.check_status(StatusFlags::SAVED)
                    & !r.check_status(StatusFlags::EXPIRED)
                    & r.check_status(StatusFlags::TRACK | StatusFlags::FETCHED)
            });
            bili.batch_pull_res(&mut sub, 8).await?;
        }
    }
    let mut sub = sets.subset(|s| s.check_status(StatusFlags::TRACK));
    for set in sub.iter_mut() {
        let mut sub = set.subset(|r| {
            !r.check_status(StatusFlags::EXPIRED)
                & r.check_status(StatusFlags::TRACK | StatusFlags::FETCHED)
                & ids.contains(&r.id())
        });
        bili.batch_pull_res(&mut sub, 8).await?;
    }

    Ok(())
}

pub(super) async fn cron(interval: u64) -> FavCoreResult<()> {
    if interval < 15 {
        warn!("Interval would better to be greater than 15 minutes.");
    }
    let duration = tokio::time::Duration::from_secs(interval * 60);
    let interval = chrono::Duration::try_minutes(interval as i64).expect("invalid interval.");

    let mut fire: bool = true;
    loop {
        tokio::select! {
            res = async {
                let mut sets = BiliSets::read().unwrap_or_default();
                pull_all(&mut sets).await?;
                sets.write()?;
                FavCoreResult::Ok(())
            }, if fire => {
                if let Err(e) = res {
                    error!("{}", e);
                }
                let next_ts_local = (chrono::Utc::now() + interval)
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
                info!(
                    "Next job will be {} minutes later at {}.\n",
                    interval.num_minutes(),
                    next_ts_local
                );
                fire = false;
            }
            _ = tokio::time::sleep(duration), if !fire => fire = true,
            _ = tokio::signal::ctrl_c(), if !fire => {
                info!("Received Ctrl-C, exiting.");
                break;
            }
        }
    }
    Ok(())
}

pub(super) fn check_ffmpeg() -> FavCoreResult<()> {
    let status = std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    if !status.success() {
        return Err(FavUtilsError::FFMPEGNotFound.into());
    }
    Ok(())
}
