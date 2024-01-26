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
    let Meta { videos, .. } = &meta();
    show_table(
        [
            "BVID",
            "Title",
            "Saved",
            "Favorited",
            "Upper",
            "Clarity",
            "ToSave",
        ],
        videos.iter().map(|v| {
            [
                v.bvid.clone(),
                v.title.chars().take(20).collect(),
                v.saved.to_string(),
                v.fav.to_string(),
                v.upper.name.clone(),
                v.clarity.as_deref().unwrap_or("default").to_owned(),
                v.to_save.to_string(),
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
