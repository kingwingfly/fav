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
    rows.into_iter()
        .inspect(|_| count += 1)
        .for_each(|r| builder.push_record(r));
    let table = builder
        .build()
        .with(Style::modern())
        .modify(Rows::new(1..), Alignment::left())
        .to_string();
    println!("{}", table);
    println!("Total: {}", count);
}
