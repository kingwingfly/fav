use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::OnceLock;

pub fn download_bar(size: usize) -> ProgressBar {
    static PBS: OnceLock<MultiProgress> = OnceLock::new();
    let pbs = PBS.get_or_init(MultiProgress::new);
    let pb = pbs.add(ProgressBar::new(size as u64));
    pb.set_style(ProgressStyle::with_template("{msg:!10} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-"));
    pb
}
