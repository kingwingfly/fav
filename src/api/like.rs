const LIKE_API: &str = "https://api.bilibili.com/x/web-interface/archive/like";

pub(crate) async fn like(id: String) {
    let url = reqwest::Url::parse_with_params(
        LIKE_API,
        [("bvid", id.as_str()), ("like", "1"), ("csrf", "")],
    )
    .unwrap();
}

pub(crate) async fn like_all() {}
