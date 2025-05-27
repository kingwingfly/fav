use std::time::Duration;

use api_req::{ApiCaller as _, COOKIE_JAR};
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use qrcode::{QrCode, render::unicode};
use tokio::time::sleep;

use crate::{
    api::AuthApi,
    event::Login,
    payload::{QrPayload, QrPollPayload},
    response::{QrData, QrResp},
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
            for _ in 0..40 {
                sleep(Duration::from_secs(3)).await;
                let resp: serde_json::Value = AuthApi::request(QrPollPayload {
                    qrcode_key: qrcode_key.clone(),
                })
                .await
                .unwrap();
                dbg!(resp);
                dbg!(COOKIE_JAR.clone());
            }
        });
    });
}
