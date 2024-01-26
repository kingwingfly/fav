use crate::{cli::utils::show_table, proto::data::Meta};

impl Meta {
    pub(crate) fn status_list(&self, tracked: bool) {
        show_table(
            ["ID", "Title", "Tracked"],
            self.lists
                .iter()
                .filter(|l| !tracked || l.is_tracked)
                .map(|l| [l.id.to_string(), l.title.clone(), l.is_tracked.to_string()]),
        );
    }

    pub(crate) fn status_video(&self) {
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
            self.videos.iter().map(|v| {
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

    pub(crate) fn status_expired(&self) {
        println!("Expired videos:");
        show_table(
            ["BVID", "Title", "Upper"],
            self.videos.iter().filter(|v| v.expired).map(|v| {
                [
                    v.bvid.to_string(),
                    v.title.chars().take(20).collect(),
                    v.upper.name.clone(),
                ]
            }),
        );
    }

    pub(crate) fn status_not_fav(&self) {
        println!("Not Favorite:");
        show_table(
            ["BVID", "Title", "Upper"],
            self.videos.iter().filter(|v| !v.fav).map(|v| {
                [
                    v.bvid.to_string(),
                    v.title.chars().take(20).collect(),
                    v.upper.name.clone(),
                ]
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::meta::meta;

    #[test]
    fn status_test() {
        meta().status_list(false);
    }
}
