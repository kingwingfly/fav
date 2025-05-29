use std::collections::HashSet;

use api_req::ApiCaller;
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use dashmap::DashSet;
use tracing::{error, info};

use crate::{
    api::BiliApi,
    cookies::{parse_cookies, set_cookie_jar},
    db::Db,
    entity::{set, set_account},
    event::Fetch,
    payload::ListSetPayload,
    response::{ListSetData, ListSetResp},
    runtime::Runtime,
    state::SetState,
};

pub fn pull(mut cmds: Commands) {
    cmds.add_observer(|_: Trigger<Fetch>, runtime: Res<Runtime>, db: Res<Db>| {
        if let Err(e) = runtime.block_on(async {
            let accounts = db.all_active_accounts().await?;
            for account in accounts.iter() {
                let account_id = account.account_id;
                info!("Pulling account<{}>", account_id);
                set_cookie_jar(parse_cookies(&account.cookies));
                let ListSetResp {
                    data: ListSetData { list },
                } = BiliApi::request(ListSetPayload { up_mid: account_id }).await?;
                let mut old_set_ids: HashSet<i64> =
                    HashSet::from_iter(db.get_set_ids_of_account(account_id).await?);
                for set in list {
                    db.upsert_set(set::Model {
                        set_id: set.id,
                        name: set.title,
                        state: SetState::Inactive.to_string(), // conflic skip
                    })
                    .await?;
                    db.upsert_set_account(set_account::Model {
                        set_id: set.id,
                        account_id,
                    })
                    .await?;
                    info!("Update set<{}>", set.id);
                    old_set_ids.remove(&set.id);
                }
                for set_id in old_set_ids {
                    db.delete_set_account(set_account::Model { set_id, account_id })
                        .await?;
                }
            }
            let done_sets = DashSet::<i64>::new();
            for account in accounts.iter() {
                let account_id = account.account_id;
                let set_ids_of_account = db.get_set_ids_of_account(account_id).await?;
                for set_id in set_ids_of_account {
                    if done_sets.contains(&set_id) {
                        continue;
                    }
                    // fetch
                    done_sets.insert(set_id);
                }
            }
            // fetch video
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e)
        }
    });
}
