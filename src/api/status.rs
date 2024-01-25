use super::error::Result;
use crate::meta::meta;
use crate::proto::data::ListMeta;
use tabled::{
    builder::Builder,
    settings::{object::Rows, Alignment, Modify, Style},
};

pub(crate) fn status() -> Result<()> {
    let lists = &meta().lists;
    let (x, y) = (lists.len(), 3);

    let mut builder = Builder::default();

    let header = ["ID", "Title", "IsTracking"];
    builder.push_record(header);

    for list in lists {
        let row = [
            list.id.to_string(),
            list.title.clone(),
            list.is_tracking.to_string(),
        ];
        builder.push_record(row);
    }

    let table = builder
        .build()
        .with(Style::rounded())
        .modify(Rows::new(1..), Alignment::left())
        .to_string();
    println!("{}", table);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_test() {
        status().unwrap();
    }
}
