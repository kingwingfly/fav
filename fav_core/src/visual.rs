//! Data visualize
use crate::attr::Attr;
use crate::res::{Res, Set, Sets};
use crate::status::{Status, StatusFlags};
use tabled::{
    builder::Builder,
    settings::{object::Rows, Alignment, Style},
};

/// Visualize the data as table
pub trait TableSets: Sets {
    /// Visualize the data as table
    fn table(&self);
}
/// Visualize the data as table
pub trait TableSet: Set {
    /// Visualize the data as table
    fn table(&self);
}
/// Visualize the data as table
pub trait TableRes: Res {
    /// Visualize the data as table
    fn table(&self);
}

impl<T> TableSets for T
where
    T: Sets,
    T::Set: Attr + Status,
{
    fn table(&self) {
        let header = vec!["ID", "Title", "Count", "Track"];
        let rows = self.iter().map(|set| {
            let id = String::from(set.id());
            let title = set.title().to_string();
            let count = set.iter().count().to_string();
            let track = set.check_status(StatusFlags::TRACK).to_string();
            vec![id, title, count, track]
        });
        show_table(header, rows);
    }
}

impl<T> TableSet for T
where
    T: Set,
    T::Res: Attr + Status,
{
    fn table(&self) {
        let header = vec!["ID", "Title", "Track", "Saved"];
        let rows = self.iter().map(|res| {
            let id = String::from(res.id());
            let title = res.title().to_string().chars().take(15).collect();
            let track = res.check_status(StatusFlags::TRACK).to_string();
            let saved = res.check_status(StatusFlags::SAVED).to_string();
            vec![id, title, track, saved]
        });
        show_table(header, rows);
    }
}

impl<T> TableRes for T
where
    T: Res,
{
    fn table(&self) {
        let header = vec!["ID", "Title", "Track", "Saved"];
        let id = String::from(self.id());
        let title = self.title().to_string().chars().take(15).collect();
        let track = self.check_status(StatusFlags::TRACK).to_string();
        let saved = self.check_status(StatusFlags::SAVED).to_string();
        let rows = vec![vec![id, title, track, saved]];
        show_table(header, rows);
    }
}

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
            println!("Count: {}\n", count);
        }
    }
}
