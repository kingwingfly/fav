use anyhow::{Context as _, anyhow};
use api_req::ApiCaller;
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use cookie::Cookie;
use futures::StreamExt;
use tracing::{error, info};

use crate::{
    api::BiliApi,
    cookies::{parse_cookies, set_cookie_jar},
    db::Db,
    event::Like,
    payload::{Buvid3Payload, LikePayload},
    response::{Buvid3Data, Buvid3Resp, LikeResp},
    runtime::Runtime,
};

pub fn like(mut cmds: Commands) {
    cmds.add_observer(
        |trigger: Trigger<Like>, runtime: Res<Runtime>, db: Res<Db>| {
            let Like { avids } = trigger.event();
            if let Err(e) = runtime.block_on(async move {
                let accounts = db.all_active_accounts().await?;
                for account in accounts {
                    let cookies = parse_cookies(&account.cookies).collect::<Vec<_>>();
                    let bili_jct = cookies
                        .iter()
                        .find(|c| c.name() == "bili_jct")
                        .map(|c| c.value().to_owned())
                        .context(format!(
                            "No bili_jct in cookies of account<{}>.",
                            account.name
                        ))?;
                    let Buvid3Resp {
                        data: Buvid3Data { buvid },
                    } = BiliApi::request(Buvid3Payload).await?;
                    set_cookie_jar([Cookie::new("buvid3", buvid)].into_iter());
                    set_cookie_jar(cookies.into_iter());
                    let mut tasks = futures::stream::iter(avids)
                        .map(|&aid| {
                            let bili_jct = bili_jct.to_owned();
                            async move {
                                let LikeResp { code, message } = BiliApi::request(LikePayload {
                                    aid,
                                    like: 1,
                                    csrf: bili_jct,
                                })
                                .await?;
                                match code {
                                    0 => {
                                        info!("Liked {}", aid);
                                        Ok::<_, anyhow::Error>(())
                                    }
                                    _ => Err(anyhow!("{}", message)),
                                }
                            }
                        })
                        .buffer_unordered(8);
                    while let Some(res) = tasks.next().await {
                        if let Err(e) = res {
                            error!("{}", e);
                        }
                    }
                }
                Ok::<_, anyhow::Error>(())
            }) {
                error!("{}", e)
            }
        },
    );
}
