use std::{
    io::{Cursor, ErrorKind, Read},
    ops::Shl,
};

use crate::{
    api::{client, error::Result},
    proto::data::Cookie,
};
use rand::Rng;
use reqwest::header::COOKIE;
use tracing::{info, warn};

const BUVID_API: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const ACTIVE_API: &str = "https://api.bilibili.com/x/internal/gaia-gateway/ExClimbWuzhi";

#[derive(serde::Deserialize)]
struct Buvids {
    #[serde(rename = "b_3")]
    buvid3: String,
    #[serde(rename = "b_4")]
    buvid4: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Payload {
    #[serde(rename = "payload")]
    inner: String,
}

impl Payload {
    fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut inner: serde_json::Value =
            serde_json::from_str(include_str!("payload.json")).unwrap();
        let ts = timestamp().to_string();
        *inner.pointer_mut("/5062").unwrap() = ts.into();
        *inner.pointer_mut("/6e7c").unwrap() =
            format!("{}x{}", rng.gen_range(800..1200), rng.gen_range(1200..3000)).into();
        Payload {
            inner: inner.to_string(),
        }
    }
}

pub(super) async fn active_buvid(cookie: &mut Cookie) -> Result<()> {
    let resp = client().get(BUVID_API).send().await?;
    let mut json: serde_json::Value = resp.json().await?;
    let buvids =
        serde_json::from_value::<Buvids>(json.pointer_mut("/data").unwrap().take()).unwrap();
    Buvids {
        buvid3: cookie.buvid3,
        buvid4: cookie.buvid4,
    } = buvids;

    cookie._uuid = uuid();

    let payload = Payload::new();
    cookie.buvid_fp = buvid_fp(&payload.inner);

    let resp = client()
        .post(ACTIVE_API)
        .header(
            COOKIE,
            format!(
                "buvid3={}; buvid4={}; _uuid={}; buvid_fp={}",
                cookie.buvid3, cookie.buvid4, cookie._uuid, cookie.buvid_fp
            ),
        )
        .json(&payload)
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    match json.pointer("/code").unwrap().as_i64().unwrap() {
        0 => info!("Actived Buvid."),
        _ => warn!(
            "Failed to active Buvid. Error Message: {}",
            json.pointer("/message").unwrap().as_str().unwrap()
        ),
    }
    println!("{:#?}", json);
    Ok(())
}

fn uuid() -> String {
    const LEN: usize = 16;
    const DIGHT_MAP: [&'static str; LEN] = [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F", "10",
    ];
    let t = timestamp() % 100_000;
    let mut rng = rand::thread_rng();
    let index: [u8; 32] = rng.gen();
    let mut result = String::with_capacity(64);
    index.into_iter().enumerate().for_each(|(ii, i)| {
        if [9, 13, 17, 21].contains(&ii) {
            result.push('-')
        };
        result.push_str(DIGHT_MAP[(i & 0x0f) as usize]);
    });
    format!("{}{}{}", result, format!("{:0>5}", t), "infoc")
}

fn buvid_fp(payload: &str) -> String {
    let tmp: u128 = murmur3_x64_128(&mut Cursor::new(payload), 31);
    format!("{:016x}{:016x}", tmp & (u64::MAX as u128), tmp >> 64)
}

fn timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

pub fn murmur3_x64_128<T: Read>(source: &mut T, seed: u32) -> u128 {
    const C1: u64 = 0x87c3_7b91_1142_53d5;
    const C2: u64 = 0x4cf5_ad43_2745_937f;
    const C3: u64 = 0x52dc_e729;
    const C4: u64 = 0x3849_5ab5;
    const R1: u32 = 27;
    const R2: u32 = 31;
    const R3: u32 = 33;
    const M: u64 = 5;
    let mut h1: u64 = seed as u64;
    let mut h2: u64 = seed as u64;
    let mut buf = [0; 16];
    let mut processed: usize = 0;
    loop {
        let read = read_bytes(source, &mut buf[..]).unwrap();
        processed += read;
        if read == 16 {
            let k1 = u64::from_le_bytes(copy_into_array(&buf[0..8]));
            let k2 = u64::from_le_bytes(copy_into_array(&buf[8..]));
            h1 ^= k1.wrapping_mul(C1).rotate_left(R2).wrapping_mul(C2);
            h1 = h1
                .rotate_left(R1)
                .wrapping_add(h2)
                .wrapping_mul(M)
                .wrapping_add(C3);
            h2 ^= k2.wrapping_mul(C2).rotate_left(R3).wrapping_mul(C1);
            h2 = h2
                .rotate_left(R2)
                .wrapping_add(h1)
                .wrapping_mul(M)
                .wrapping_add(C4);
        } else if read == 0 {
            h1 ^= processed as u64;
            h2 ^= processed as u64;
            h1 = h1.wrapping_add(h2);
            h2 = h2.wrapping_add(h1);
            h1 = fmix64(h1);
            h2 = fmix64(h2);
            h1 = h1.wrapping_add(h2);
            h2 = h2.wrapping_add(h1);
            let x = ((h2 as u128) << 64) | (h1 as u128);
            return x;
        } else {
            let mut k1 = 0;
            let mut k2 = 0;
            if read >= 15 {
                k2 ^= (buf[14] as u64).shl(48);
            }
            if read >= 14 {
                k2 ^= (buf[13] as u64).shl(40);
            }
            if read >= 13 {
                k2 ^= (buf[12] as u64).shl(32);
            }
            if read >= 12 {
                k2 ^= (buf[11] as u64).shl(24);
            }
            if read >= 11 {
                k2 ^= (buf[10] as u64).shl(16);
            }
            if read >= 10 {
                k2 ^= (buf[9] as u64).shl(8);
            }
            if read >= 9 {
                k2 ^= buf[8] as u64;
                k2 = k2.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
                h2 ^= k2;
            }
            if read >= 8 {
                k1 ^= (buf[7] as u64).shl(56);
            }
            if read >= 7 {
                k1 ^= (buf[6] as u64).shl(48);
            }
            if read >= 6 {
                k1 ^= (buf[5] as u64).shl(40);
            }
            if read >= 5 {
                k1 ^= (buf[4] as u64).shl(32);
            }
            if read >= 4 {
                k1 ^= (buf[3] as u64).shl(24);
            }
            if read >= 3 {
                k1 ^= (buf[2] as u64).shl(16);
            }
            if read >= 2 {
                k1 ^= (buf[1] as u64).shl(8);
            }
            if read >= 1 {
                k1 ^= buf[0] as u64;
            }
            k1 = k1.wrapping_mul(C1);
            k1 = k1.rotate_left(31);
            k1 = k1.wrapping_mul(C2);
            h1 ^= k1;
        }
    }
}

#[inline]
fn fmix64(k: u64) -> u64 {
    const C1: u64 = 0xff51_afd7_ed55_8ccd;
    const C2: u64 = 0xc4ce_b9fe_1a85_ec53;
    const R: u32 = 33;
    let mut tmp = k;
    tmp ^= tmp >> R;
    tmp = tmp.wrapping_mul(C1);
    tmp ^= tmp >> R;
    tmp = tmp.wrapping_mul(C2);
    tmp ^= tmp >> R;
    tmp
}

#[inline]
fn copy_into_array<A, T>(slice: &[T]) -> A
where
    A: Default + AsMut<[T]>,
    T: Copy,
{
    let mut a = A::default();
    <A as AsMut<[T]>>::as_mut(&mut a).copy_from_slice(slice);
    a
}

/// Try to fill buf with data from source, dealing with short reads such as
/// caused by Chain.
///
/// Errors: See `std::io::Read`.
#[inline]
fn read_bytes<R>(source: &mut R, buf: &mut [u8]) -> std::io::Result<usize>
where
    R: Read,
{
    let mut offset = 0;
    loop {
        match source.read(&mut buf[offset..]) {
            Ok(0) => {
                return Ok(offset);
            }
            Ok(n) => {
                offset += n;
                if offset == buf.len() {
                    return Ok(offset);
                }
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => {
                return Err(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn active_buvid_test() {
        let mut cookie = Cookie::default();
        assert!(active_buvid(&mut cookie).await.is_ok());
    }

    #[test]
    fn uuid_test() {
        println!("{}", uuid());
    }

    #[test]
    fn buvid_fp_test() {
        let payload = Payload::new();
        println!("{}", buvid_fp(&payload.inner));
    }
}
