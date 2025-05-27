use std::time::Duration;

use api_req::{ApiCaller as _, COOKIE_JAR, CookieStore as _};
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use qrcode::{QrCode, render::unicode};
use tokio::time::sleep;

use crate::{
    api::{AuthApi, BiliApi},
    db::Db,
    event::Login,
    payload::{QrPayload, QrPollPayload, WbiPayload},
    response::{QrData, QrPollData, QrPollResp, QrResp, WbiData, WbiResp},
    runtime::Runtime,
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
                    data: WbiData { uname, .. },
                } = BiliApi::request(WbiPayload).await.unwrap();
                println!("Hello😊, {}.", uname);
            });
        },
    );
}
