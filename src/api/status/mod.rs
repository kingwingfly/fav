mod query;

use crate::{
    cli::utils::show_table,
    proto::data::{Clarity, Meta},
};

impl Meta {
    pub(crate) fn status_list(&self, tracked: bool) {
        println!("Lists:");
        show_table(
            ["ID", "Title", "Count", "Track"],
            self.lists.iter().filter(|l| !tracked || l.track).map(|l| {
                [
                    l.id.to_string(),
                    l.title.clone(),
                    l.media_count.to_string(),
                    l.track.to_string(),
                ]
            }),
        );
    }

    pub(crate) fn status_video(&self, tracked: bool) {
        println!("Videos:");
        show_table(
            [
                "BVID", "Title", "Saved", "Fave", "Upper", "Clarity", "Track", "Expired",
            ],
            self.videos.iter().filter(|v| !tracked || v.track).map(|v| {
                [
                    v.bvid.clone(),
                    v.title.chars().take(20).collect(),
                    v.saved.to_string(),
                    v.fav.to_string(),
                    v.upper.name.clone(),
                    v.clarity.unwrap().to_string(),
                    v.track.to_string(),
                    v.expired.to_string(),
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

impl ToString for Clarity {
    fn to_string(&self) -> String {
        match self {
            Clarity::FourK => "4k",
            Clarity::FullHDHighFrame => "1080P60",
            Clarity::FullHDHighCode => "1080P+",
            Clarity::FullHD => "1080P",
            Clarity::HDHighFrame => "720P60",
            Clarity::HD => "720P",
            Clarity::SD => "480P",
            Clarity::LD => "360P",
            Clarity::VLD => "240P",
        }
        .to_string()
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
