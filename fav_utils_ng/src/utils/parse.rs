use fav_core::FavCoreResult;
use protobuf::MessageFull;
use protobuf_json_mapping::{parse_from_str_with_options, ParseOptions};
use reqwest::Response;
use serde::de::DeserializeOwned;
use serde_json::Value;

const PARSE_OPTIONS: ParseOptions = ParseOptions {
    ignore_unknown_fields: true,
    _future_options: (),
};

pub async fn resp2serde<T: DeserializeOwned>(resp: Response, pointer: &str) -> FavCoreResult<T> {
    Ok(serde_json::from_value(
        resp.json::<Value>()
            .await?
            .pointer_mut(pointer)
            .unwrap()
            .take(),
    )?)
}

pub async fn resp2proto<T: MessageFull>(resp: Response, pointer: &str) -> FavCoreResult<T> {
    let json = resp
        .json::<Value>()
        .await?
        .pointer_mut(pointer)
        .unwrap()
        .take();
    Ok(parse_from_str_with_options(
        &json.to_string(),
        &PARSE_OPTIONS,
    )?)
}
