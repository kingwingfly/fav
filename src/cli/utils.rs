use clap::{builder::PossibleValue, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::OnceLock;
use tabled::{
    builder::Builder,
    settings::{object::Rows, Alignment, Style},
};

use crate::proto::data::Qn;

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

pub(crate) fn download_bar(size: usize) -> ProgressBar {
    static PBS: OnceLock<MultiProgress> = OnceLock::new();
    let pbs = PBS.get_or_init(MultiProgress::new);
    let pb = pbs.add(ProgressBar::new(size as u64));
    pb.set_style(ProgressStyle::with_template("{msg:!10} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-"));
    pb
}

impl ValueEnum for Qn {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Default,
            Self::EightK,
            Self::Dolby,
            Self::HDR,
            Self::FourK,
            Self::FullHDHighFrame,
            Self::FullHDHighCode,
            Self::FullHD,
            Self::HDHighFrame,
            Self::HD,
            Self::SD,
            Self::LD,
            Self::VLD,
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Qn::Default => Some(PossibleValue::new("default")),
            Qn::EightK => Some(PossibleValue::new("8k")),
            Qn::Dolby => Some(PossibleValue::new("dolby")),
            Qn::HDR => Some(PossibleValue::new("hdr")),
            Qn::FourK => Some(PossibleValue::new("4k")),
            Qn::FullHDHighFrame => Some(PossibleValue::new("1080p60")),
            Qn::FullHDHighCode => Some(PossibleValue::new("1080p+")),
            Qn::FullHD => Some(PossibleValue::new("1080p")),
            Qn::HDHighFrame => Some(PossibleValue::new("720p60")),
            Qn::HD => Some(PossibleValue::new("720p")),
            Qn::SD => Some(PossibleValue::new("480p")),
            Qn::LD => Some(PossibleValue::new("360p")),
            Qn::VLD => Some(PossibleValue::new("240p")),
        }
    }
}
