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
    event::Login,
    payload::{QrPayload, QrPollPayload, WhoamiPayload},
    response::{QrData, QrPollData, QrPollResp, QrResp, WhoamiData, WhoamiResp},
    runtime::Runtime,
};

pub fn auth(mut cmds: Commands) {
    cmds.add_observer(move |_: Trigger<Login>, runtime: Res<Runtime>| {
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
            let WhoamiResp {
                data: WhoamiData { name, .. },
            } = BiliApi::request(WhoamiPayload::new(32280488).await)
                .await
                .unwrap();
            println!("Hello {}", name);
        });
    });
}
