use tabled::{Table, builder::Builder, settings::Style};

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
