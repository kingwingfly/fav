use std::time::Duration;

use api_req::{ApiCaller as _, COOKIE_JAR, CookieStore as _};
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use cookie::Cookie;
use qrcode::{QrCode, render::unicode};
use tokio::time::sleep;

use crate::{
    api::{AuthApi, BiliApi},
    cookies::{parse_cookies, set_cookie_jar},
    db::Db,
    entity::{ToTableRecord, user},
    event::{ListUser, Login, Logout},
    payload::{LogoutPayload, QrPayload, QrPollPayload, WbiPayload},
    response::{LogoutResp, QrData, QrPollData, QrPollResp, QrResp, WbiData, WbiResp},
    runtime::Runtime,
    state::UserState,
    table::table,
};

pub fn auth(mut cmds: Commands) {
    cmds.add_observer(
        move |_: Trigger<Login>, runtime: Res<Runtime>, db: Res<Db>| {
            runtime.block_on(async {
                let QrResp {
                    data: QrData { url, qrcode_key },
                } = AuthApi::request(QrPayload).await.unwrap();
                let code = QrCode::new(url.as_ref()).unwrap();
                let image = code
                    .render::<unicode::Dense1x2>()
                    .dark_color(unicode::Dense1x2::Light)
                    .light_color(unicode::Dense1x2::Dark)
                    .build();
                println!("{}", image);
                loop {
                    sleep(Duration::from_secs(3)).await;
                    let QrPollResp {
                        data: QrPollData { code, message },
                    } = AuthApi::request(QrPollPayload {
                        qrcode_key: qrcode_key.clone(),
                    })
                    .await
                    .unwrap();
                    match code {
                        0 => {
                            println!("Login successfully.");
                            break;
                        }
                        86101 | 86090 => {}
                        _ => {
                            println!("{}", message);
                            return;
                        }
                    }
                }
                let WbiResp {
                    data: WbiData { mid, uname, .. },
                } = BiliApi::request(WbiPayload).await.unwrap();
                let cookies = COOKIE_JAR
                    .cookies(&"https://bilibili.com".parse().unwrap())
                    .expect("Auth related cookies should be set by bilibili.")
                    .to_str()
                    .unwrap()
                    .to_owned();
                db.upsert_user(user::Model {
                    user_id: mid,
                    name: uname.to_owned(),
                    cookies,
                    state: UserState::Active.to_string(),
                })
                .await;
                println!("Hello😊, {}.", uname);
            });
        },
    );
    cmds.add_observer(
        |trigger: Trigger<Logout>, runtime: Res<Runtime>, db: Res<Db>| {
            runtime.block_on(async {
                let Logout { user_id } = *trigger;
                let user::Model { name, cookies, .. } = db.get_user(user_id).await;
                let cookies = parse_cookies(cookies).collect::<Vec<_>>();
                let bili_jct = cookies
                    .iter()
                    .find(|c| c.name() == "bili_jct")
                    .map(|c| c.value().to_owned())
                    .unwrap();
                set_cookie_jar(cookies.into_iter());
                let LogoutResp { code, message } =
                    AuthApi::request(LogoutPayload { biliCSRF: bili_jct })
                        .await
                        .unwrap();
                match code {
                    0 => {
                        db.delete_user(user_id).await;
                        println!("Goodbye👋, {}", name);
                    }
                    _ => println!("Failed to logout: {}", message.unwrap_or_default()),
                }
            });
        },
    );
    cmds.add_observer(|_: Trigger<ListUser>, runtime: Res<Runtime>, db: Res<Db>| {
        runtime.block_on(async {
            let users = db.all_users().await;
            println!(
                "{}",
                table(
                    ["user id", "name", "state"],
                    users.into_iter().map(ToTableRecord::to_record)
                )
            )
        });
    });
}
