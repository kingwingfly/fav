use std::collections::HashSet;

use api_req::ApiCaller;
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use dashmap::DashSet;
use futures::StreamExt as _;
use tracing::{error, info, warn};

use crate::{
    api::BiliApi,
    cookies::{parse_cookies, set_cookie_jar},
    db::Db,
    entity::{account, media, media_set, set, set_account},
    event::{Fetch, ListMedia},
    payload::{InSetPayload, ListSetPayload, MediaInfoPayload},
    response::{InSetData, InSetResp, ListSetData, ListSetResp, MediaInfoResp},
    runtime::Runtime,
    state::{MediaState, SetState},
};

pub fn pull(mut cmds: Commands) {
    cmds.add_observer(|_: Trigger<Fetch>, runtime: Res<Runtime>, db: Res<Db>| {
        if let Err(e) = runtime.block_on(async {
            let accounts = db.all_active_accounts().await?;
            for account in accounts.iter() {
                info!("Fetching sets with account<{}>", account.name);
                set_cookie_jar(parse_cookies(&account.cookies));
                let account_id = account.account_id;
                let ListSetResp {
                    data: ListSetData { list },
                } = BiliApi::request(ListSetPayload { up_mid: account_id }).await?;
                let mut old_set_ids: HashSet<i64> =
                    HashSet::from_iter(db.get_set_ids_of_account(account_id).await?);
                db.upsert_sets(list.iter().map(|set| {
                    info!("Updating set<{}>", set.title);
                    set::Model {
                        set_id: set.id,
                        name: set.title.to_owned(),
                        count: set.media_count,
                        state: SetState::Inactive.to_string(), // conflic skip
                    }
                }))
                .await?;
                db.upsert_set_accounts(list.iter().map(|set| {
                    info!("Linking set<{}> and account<{}>", set.title, account.name);
                    set_account::Model {
                        set_id: set.id,
                        account_id,
                    }
                }))
                .await?;
                for set in list {
                    old_set_ids.remove(&set.id);
                }
                for set_id in old_set_ids {
                    db.delete_set_account(set_account::Model { set_id, account_id })
                        .await?;
                    warn!("Unlinked set<{}> and account<{}>", set_id, account_id);
                }
            }
            let fetched_sets = DashSet::<i64>::new();
            for account in accounts.iter() {
                info!("Fetching set contents with account<{}>", account.name);
                set_cookie_jar(parse_cookies(&account.cookies));
                let account_id = account.account_id;
                let set_ids_of_account = db.get_set_ids_of_account(account_id).await?;
                for set_id in set_ids_of_account {
                    if fetched_sets.contains(&set_id) {
                        continue;
                    }
                    let set = db.get_set(set_id).await?;
                    if set.state != SetState::Active.to_string() || set.count == 0 {
                        continue;
                    }
                    let page = (set.count - 1) / 20 + 1;
                    let mut tasks = futures::stream::iter(1..=page)
                        .map(|pn| async move {
                            let InSetResp {
                                data: InSetData { medias },
                            } = BiliApi::request(InSetPayload {
                                media_id: set.set_id,
                                pn,
                                ps: 20,
                            })
                            .await?;
                            Ok::<_, anyhow::Error>(medias)
                        })
                        .buffer_unordered(8);
                    let mut medias = vec![];
                    while let Some(res) = tasks.next().await {
                        match res {
                            Ok(list) => medias.extend(list),
                            Err(e) => error!("{}", e),
                        }
                    }
                    db.upsert_medias(medias.iter().map(|m| {
                        info!("Updating media<{}>", m.title);
                        media::Model {
                            id: m.id,
                            bv_id: m.bv_id.to_owned(),
                            title: m.title.to_owned(),
                            r#type: m.r#type.to_string(),
                            state: MediaState::Pending.to_string(),
                        }
                    }))
                    .await?;
                    db.upsert_media_sets(medias.into_iter().map(|m| {
                        info!("Linking media<{}> and set<{}>", m.title, set.name);
                        media_set::Model { id: m.id, set_id }
                    }))
                    .await?;
                    fetched_sets.insert(set_id);
                }
            }
            let fetched_ups = DashSet::<i64>::new();
            let fetched_medias = DashSet::<i64>::new();
            for account in accounts.iter() {
                info!("Fetching medias with account<{}>", account.name);
                set_cookie_jar(parse_cookies(&account.cookies));
                let account_id = account.account_id;
                let medias = db.all_medias().await?;
                for media in medias {
                    let resp: MediaInfoResp =
                        BiliApi::request(MediaInfoPayload { aid: media.id }).await?;
                    dbg!(resp);
                }
            }
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e)
        }
    });
}
