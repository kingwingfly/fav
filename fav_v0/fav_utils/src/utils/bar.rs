use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::{cmp::Ordering, sync::OnceLock};
use unicode_width::UnicodeWidthChar;

pub fn download_bar(size: usize, title: impl AsRef<str>) -> ProgressBar {
    static PBS: OnceLock<MultiProgress> = OnceLock::new();
    let pbs = PBS.get_or_init(MultiProgress::new);
    let pb = pbs.add(ProgressBar::new(size as u64));
    pb.set_message(take_by_unicode_width(title.as_ref(), 10));
    pb.set_style(ProgressStyle::with_template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-"));
    pb
}

fn take_by_unicode_width(s: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut res = String::new();
    for ch in s.chars() {
        let ch_width = ch.width().unwrap_or_default();
        match (width + ch_width).cmp(&max_width) {
            Ordering::Less | Ordering::Equal => res.push(ch),
            Ordering::Greater => break,
        }
        width += ch_width;
    }
    if width < max_width {
        res.push_str(&" ".repeat(max_width - width));
    }
    res
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_take_by_unicode_width() {
        assert_eq!(take_by_unicode_width("a", 1), "a");
        assert_eq!(take_by_unicode_width("中", 1), " ");
        assert_eq!(take_by_unicode_width("中", 2), "中");
        assert_eq!(take_by_unicode_width("中", 3), "中 ");
        assert_eq!(take_by_unicode_width("中aa", 4), "中aa");
        assert_eq!(take_by_unicode_width("中aa", 5), "中aa ");
        assert_eq!(take_by_unicode_width("中国人不骗中国人", 6), "中国人");
    }

    #[test]
    #[ignore = "This test is for manual inspection"]
    fn test_download_bar() {
        fn download_bar_job(title: impl AsRef<str>) {
            let pb = download_bar(100, title);
            for _ in 0..100 {
                pb.inc(1);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            pb.finish();
        }
        let jhs: Vec<_> = [
            "中国人不骗中国人中国人不骗中国人",
            "中国人不骗中国人",
            "Rust起飞啦",
            "Rust",
            "中",
        ]
        .into_iter()
        .map(|title| std::thread::spawn(move || download_bar_job(title)))
        .collect();
        for jh in jhs {
            jh.join().unwrap();
        }
    }
}
