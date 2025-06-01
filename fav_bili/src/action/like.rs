use std::collections::HashMap;

use anyhow::{Context as _, Result, anyhow};
use api_req::ApiCaller;
use cookie::Cookie;
use futures::StreamExt;
use sea_orm::ColumnTrait as _;
use tracing::{error, info, warn};

use crate::{
    api::BiliApi,
    cookies::{add_cookie_jar, current_cookies, parse_cookies},
    db::db,
    entity::account,
    payload::{Buvid3Payload, LikePayload, TicketPayload},
    response::{Buvid3Data, Buvid3Resp, LikeResp, TicketData, TicketResp},
    state::AccountState,
};

pub async fn like(avids: Vec<i64>) -> Result<()> {
    let db = db().await;
    let accounts = db
        .get_accounts_filtered(account::Column::State.eq(AccountState::Active))
        .await?;
    for account in accounts {
        let mut cookies = parse_cookies(&account.cookies)
            .map(|c| (c.name().to_owned(), c))
            .collect::<HashMap<_, _>>();
        let bili_jct = cookies
            .get("bili_jct")
            .map(|c| c.value().to_owned())
            .context(format!(
                "No bili_jct in cookies of account<{}>.",
                account.name
            ))?;
        match (
            cookies
                .get("bili_ticket_expires")
                .and_then(|c| c.value().parse::<u64>().ok()),
            cookies.contains_key("bili_ticket"),
        ) {
            (Some(bili_ticket_expires), true)
                if bili_ticket_expires
                    > std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() => {}
            (None, true) => {
                warn!(
                    "bili_ticket_expires not exist or invalid, did not check bili_ticket account<{}>",
                    account.name
                )
            }
            _ => {
                warn!(
                    "bili_ticket has expired or not exists account<{}>",
                    account.name
                );
                info!("generating bili_ticket account<{}>", account.name);
                let TicketResp {
                    data:
                        TicketData {
                            ticket,
                            created_at,
                            ttl,
                        },
                } = BiliApi::request(TicketPayload::new(bili_jct.to_owned())).await?;
                cookies.extend(
                    [
                        Cookie::new("bili_ticket", ticket),
                        Cookie::new("bili_ticket_expires", (created_at + ttl).to_string()),
                    ]
                    .into_iter()
                    .map(|c| (c.value().to_owned(), c)),
                );
            }
        }
        if !cookies.contains_key("buvid3") {
            info!("generating buvid3 account<{}>", account.name);
            let Buvid3Resp {
                data: Buvid3Data { buvid },
            } = BiliApi::request(Buvid3Payload).await?;
            cookies.insert("buvid3".to_string(), Cookie::new("buvid3", buvid));
        }
        add_cookie_jar(cookies.into_values());
        info!("Saving cookies account<{}>", account.name);
        let cookies = current_cookies()?;
        db.upsert_account(account::Model {
            account_id: account.account_id,
            name: account.name,
            cookies,
            state: account.state,
        })
        .await?;
        let mut tasks = futures::stream::iter(avids.iter())
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
    Ok(())
}
