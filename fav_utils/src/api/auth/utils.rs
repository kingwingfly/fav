use base64::Engine;
use rand::Rng;
use std::{
    io::{ErrorKind, Read},
    ops::Shl,
};

pub(super) fn webgl_str() -> String {
    let mut rng = rand::thread_rng();
    let mut rand_png_end = rng.gen::<[u8; 32]>().to_vec();
    rand_png_end.extend_from_slice(&[0; 4]);
    rand_png_end.extend_from_slice(b"IEND");
    rand_png_end.extend_from_slice(&rng.gen::<[u8; 4]>());
    let encoded = base64::prelude::BASE64_STANDARD.encode(&rand_png_end);
    encoded.split_at(encoded.len() - 50).1.to_owned()
}

pub(super) fn murmur3_x64_128<T: Read>(source: &mut T, seed: u32) -> u128 {
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
