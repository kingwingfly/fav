use fav_core::ops::{ResOpsExt, SetOpsExt};
use fav_core::prelude::*;
use fav_core::status::SetStatusExt;
use fav_core::visual::{TableRes, TableSet, TableSets};
use fav_utils::bili::{Bili, BiliRes, BiliSet, BiliSets};
use tracing::{info, warn};

pub(super) fn init() -> FavCoreResult<()> {
    #[cfg(not(test))]
    std::fs::create_dir_all(".fav")?;
    BiliSets::default().write()?;
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

pub(super) fn status(id: String) -> FavCoreResult<()> {
    let mut sets = BiliSets::read()?;
    let id_ = Id::from(&id);
    if let Some(s) = try_find_set(&mut sets, &id_) {
        s.table();
    } else if let Some(r) = try_find_res(&mut sets, &id_) {
        r.table();
    } else {
        return Err(FavCoreError::IdNotUsable(id));
    }
    Ok(())
}

pub(super) fn status_all(sets: bool, res: bool, track: bool) -> FavCoreResult<()> {
    let mut sets_ = BiliSets::read()?;
    if sets {
        let sub = sets_.subset(|s| s.check_status(StatusFlags::TRACK) | !track);
        sub.table();
    }
    if res {
        for set in sets_.iter_mut() {
            let sub = set.subset(|r| r.check_status(StatusFlags::TRACK) | !track);
            sub.table();
        }
    }
    Ok(())
}

pub(super) async fn fetch() -> FavCoreResult<()> {
    let bili = Bili::read()?;
    let mut sets = BiliSets::read()?;
    bili.fetch_sets(&mut sets).await?;
    let mut sub = sets.subset(|s| s.check_status(StatusFlags::TRACK));
    bili.batch_fetch_set(&mut sub).await?;
    for set in sub.iter_mut() {
        let mut sub = set.subset(|r| {
            r.check_status(StatusFlags::TRACK)
                & !r.check_status(StatusFlags::FETCHED)
                & !r.check_status(StatusFlags::EXPIRED)
        });
        bili.batch_fetch_res(&mut sub).await?;
    }
    sets.write()
}

pub(super) fn track(id: String) -> FavCoreResult<()> {
    let mut sets = BiliSets::read()?;
    let id_ = Id::from(&id);
    if let Some(s) = try_find_set(&mut sets, &id_) {
        s.on_status(StatusFlags::TRACK);
        s.on_res_status(StatusFlags::TRACK);
    } else if let Some(r) = try_find_res(&mut sets, &id_) {
        r.on_status(StatusFlags::TRACK);
    } else {
        return Err(FavCoreError::IdNotUsable(id));
    }
    sets.write()
}

pub(super) fn untrack(id: String) -> FavCoreResult<()> {
    let mut sets = BiliSets::read()?;
    let id_ = Id::from(&id);
    if let Some(s) = try_find_set(&mut sets, &id_) {
        s.off_status(StatusFlags::TRACK);
        s.medias.clear();
    } else if let Some(r) = try_find_res(&mut sets, &id_) {
        r.off_status(StatusFlags::TRACK);
    } else {
        return Err(FavCoreError::IdNotUsable(id));
    }
    sets.write()
}

pub(super) async fn pull_all() -> FavCoreResult<()> {
    fetch().await?;
    let bili = Bili::read()?;
    let mut sets = BiliSets::read()?;
    let mut sub = sets.subset(|s| s.check_status(StatusFlags::TRACK));
    for set in sub.iter_mut() {
        let mut sub = set.subset(|r| {
            !r.check_status(StatusFlags::SAVED)
                & !r.check_status(StatusFlags::EXPIRED)
                & r.check_status(StatusFlags::TRACK | StatusFlags::FETCHED)
        });
        bili.batch_pull_res(&mut sub).await?;
    }
    sets.write()
}

pub(super) async fn pull(id: String) -> FavCoreResult<()> {
    fetch().await?;
    let bili = Bili::read()?;
    let mut sets = BiliSets::read()?;
    let id_ = Id::from(&id);
    if let Some(s) = try_find_set(&mut sets, &id_) {
        let mut sub = s.subset(|r| {
            !r.check_status(StatusFlags::SAVED)
                & !r.check_status(StatusFlags::EXPIRED)
                & r.check_status(StatusFlags::TRACK | StatusFlags::FETCHED)
        });
        bili.batch_pull_res(&mut sub).await?;
    } else if let Some(r) = try_find_res(&mut sets, &id_) {
        if !r.check_status(StatusFlags::EXPIRED)
            & r.check_status(StatusFlags::TRACK | StatusFlags::FETCHED)
        {
            bili.pull_res(r).await?;
        }
    } else {
        return Err(FavCoreError::IdNotUsable(id));
    }
    sets.write()
}

pub(super) async fn daemon(interval: u64) {
    if interval < 15 {
        warn!("Interval would better to be greater than 15 minutes.");
    }
    pull_all().await.ok();
    loop {
        let next_ts_local = (chrono::Utc::now()
            + chrono::Duration::try_minutes(interval as i64).expect("invalid interval."))
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
        info!(
            "Next job will be {} minutes later at {}.\n",
            interval, next_ts_local
        );
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(interval * 60)) => {
                pull_all().await.ok();
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl-C, exiting.");
                break;
            }
        }
    }
}

fn try_find_set<'a>(sets: &'a mut BiliSets, id: &Id) -> Option<&'a mut BiliSet> {
    sets.iter_mut().find(|s| s.id() == *id)
}

fn try_find_res<'a>(sets: &'a mut BiliSets, id: &Id) -> Option<&'a mut BiliRes> {
    for set in sets.iter_mut() {
        if let Some(r) = set.iter_mut().find(|r| r.id() == *id) {
            return Some(r);
        }
    }
    None
}
