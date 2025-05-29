use api_req::ApiCaller;
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use tracing::error;

use crate::{
    api::BiliApi,
    cookies::{parse_cookies, set_cookie_jar},
    db::Db,
    entity::{set, set_account},
    event::{ListSet, PullMeta},
    payload::ListSetPayload,
    response::{ListSetData, ListSetResp},
    runtime::Runtime,
    state::SetState,
};

pub fn pull(mut cmds: Commands) {
    cmds.add_observer(
        |_: Trigger<PullMeta>, mut cmds: Commands, runtime: Res<Runtime>, db: Res<Db>| {
            if let Err(e) = runtime.block_on(async {
                let accounts = db.all_active_accounts().await?;
                for account in accounts {
                    let old_sets_of_acount = db.get_sets_of_account(account.account_id).await?;
                    set_cookie_jar(parse_cookies(account.cookies));
                    let ListSetResp {
                        data: ListSetData { list },
                    } = BiliApi::request(ListSetPayload {
                        up_mid: account.account_id,
                    })
                    .await?;
                    for set in old_sets_of_acount
                        .into_iter()
                        .filter(|old| list.iter().all(|new| new.id != old.set_id))
                    {
                        db.delete_set(set.set_id).await?;
                    }
                    for set in list {
                        db.upsert_set(set::Model {
                            set_id: set.id,
                            name: set.title,
                            state: SetState::Inactive.to_string(),
                        })
                        .await?;
                        db.upsert_set_account(set_account::Model {
                            set_id: set.id,
                            account_id: account.account_id,
                        })
                        .await?;
                    }
                }
                cmds.trigger(ListSet);
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e);
            };
        },
    );
}
