use indicatif::{ProgressBar, ProgressStyle};
use tabled::{
    builder::Builder,
    settings::{object::Rows, Alignment, Style},
};

pub(crate) fn show_table<H, R>(header: H, rows: R)
where
    H: IntoIterator,
    H::Item: Into<String>,
    R: IntoIterator,
    R::Item: IntoIterator,
    <R::Item as IntoIterator>::Item: Into<String>,
{
    let mut builder = Builder::default();
    builder.push_record(header);
    let mut count = 0;
    rows.into_iter().for_each(|r| {
        count += 1;
        builder.push_record(r);
    });
    match count {
        0 => {}
        _ => {
            let table = builder
                .build()
                .with(Style::modern())
                .modify(Rows::new(1..), Alignment::left())
                .to_string();
            println!("{}", table);
        }
    }
    println!("Count: {}\n", count);
}

pub(crate) fn download_bar(size: i64) -> ProgressBar {
    let pb = ProgressBar::new(size as u64);
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-"));
    pb
}
