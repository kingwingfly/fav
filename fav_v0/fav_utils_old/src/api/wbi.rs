use super::timestamp_sc;
use crate::api::client;
use serde::Deserialize;

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];
const NAV_API: &str = "https://api.bilibili.com/x/web-interface/nav";

#[derive(Deserialize)]
struct WbiImg {
    img_url: String,
    sub_url: String,
}

#[derive(Deserialize)]
struct Data {
    wbi_img: WbiImg,
}

#[derive(Deserialize)]
struct ResWbi {
    data: Data,
}

fn get_mixin_key(orig: &[u8]) -> String {
    MIXIN_KEY_ENC_TAB
        .iter()
        .map(|&i| orig[i] as char)
        .collect::<String>()
}

fn get_url_encoded(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c.is_ascii_alphanumeric() || "-_.~".contains(c) {
            true => Some(c.to_string()),
            false => {
                // 过滤 value 中的 "!'()*" 字符
                if "!'()*".contains(c) {
                    return None;
                }
                let encoded = c
                    .encode_utf8(&mut [0; 4])
                    .bytes()
                    .fold("".to_string(), |acc, b| acc + &format!("%{:02X}", b));
                Some(encoded)
            }
        })
        .collect::<String>()
}

pub(super) fn encode_wbi(
    params: &mut Vec<(&str, String)>,
    (img_key, sub_key): (String, String),
) -> String {
    let mixin_key = get_mixin_key((img_key + &sub_key).as_bytes());
    let cur_time = timestamp_sc();
    // 添加当前时间戳
    params.push(("wts", cur_time.to_string()));
    // 重新排序
    params.sort_by(|a, b| a.0.cmp(b.0));
    let query = params.iter().fold(String::from(""), |acc, (k, v)| {
        acc + format!("{}={}&", get_url_encoded(k), get_url_encoded(v)).as_str()
    });

    let web_sign = format!("{:?}", md5::compute(query.clone() + &mixin_key));

    query + &format!("w_rid={}", web_sign)
}

pub(super) async fn get_wbi_keys() -> Result<(String, String), reqwest::Error> {
    let ResWbi {
        data: Data { wbi_img },
    } = client().get(NAV_API).send().await?.json::<ResWbi>().await?;

    Ok((wbi_img.img_url, wbi_img.sub_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wbi_test() {
        let wbi_keys = get_wbi_keys().await.unwrap();
        let mut params = vec![
            ("a", "1".to_string()),
            ("b", "2".to_string()),
            ("c", "3".to_string()),
        ];
        let encoded = encode_wbi(&mut params, wbi_keys);
        println!("{}", encoded);
    }
}
