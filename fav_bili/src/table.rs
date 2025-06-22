use std::iter::repeat;

use tabled::{Table, builder::Builder, settings::Style};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

pub fn table<H, RS, R>(header: H, records: RS) -> Table
where
    H: IntoIterator,
    H::Item: Into<String>,
    RS: IntoIterator<Item = R>,
    R: IntoIterator,
    R::Item: Into<String>,
{
    let mut table = Builder::new();
    table.push_record(header);
    for record in records.into_iter() {
        table.push_record(record);
    }
    let mut table = table.build();
    table.with(Style::markdown());
    table
}

/// Return head len sub-string of s (unicode-width)
pub fn head(s: String, len: usize) -> String {
    if s.width_cjk() <= len {
        return s;
    }
    let mut n = 0;
    let mut cur = 0;
    for c in s.chars() {
        cur += c.width_cjk().unwrap_or_default();
        if cur > len {
            break;
        }
        n += 1;
    }
    s.chars().chain(repeat(' ')).take(n).collect()
}
