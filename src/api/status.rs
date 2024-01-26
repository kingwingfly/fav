use std::ops::Deref;

use crate::{cli::utils::show_table, meta::meta, proto::data::Meta};

pub(crate) fn status_list() {
    let lists = &meta().lists;
    show_table(
        ["ID", "Title", "Tracked"],
        lists
            .iter()
            .map(|l| [l.id.to_string(), l.title.clone(), l.is_tracked.to_string()]),
    );
}

pub fn status_video() {
    let Meta {
        sav_and_fav,
        sav_but_unfav,
        unsav_but_fav,
        unsav_anymore,
        ..
    } = &meta();
    show_table(
        ["BVID", "Title", "Saved", "Favorited", "Upper", "Clarity"],
        sav_and_fav.iter().map(|v| {
            [
                v.bvid.deref(),
                v.title.deref(),
                "Yes",
                "Yes",
                v.upper.name.deref(),
                v.clarity.as_deref().unwrap_or("default"),
            ]
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_test() {
        status_list();
    }
}
