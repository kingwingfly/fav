use crate::{
    cli::utils::show_table,
    proto::data::{Meta, Qn},
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

    pub(crate) fn status_of(&self, id: String) {
        if let Some(list) = self
            .lists
            .iter()
            .find(|l| l.id == id.parse::<i64>().unwrap_or_default())
        {
            println!("List<{}> Status:", list.title);
            show_table(
                ["Id", "Title", "Count", "Expired", "Track"],
                [[
                    list.id.to_string(),
                    list.title.chars().take(20).collect(),
                    list.media_count.to_string(),
                    list.expired.to_string(),
                    list.track.to_string(),
                ]],
            );
            println!("Videos in list<{}>:", list.title);
            show_table(
                [
                    "BVID", "Title", "Saved", "Fave", "Upper", "Clarity", "Track", "Expired",
                ],
                self.videos
                    .iter()
                    .filter(|v| v.list_ids.contains(&list.id))
                    .map(|v| {
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
        if let Some(video) = self.videos.iter().find(|v| v.bvid == id) {
            println!("Video<{}> Status:", video.bvid);
            show_table(
                [
                    "BVID", "Title", "Saved", "Fave", "Upper", "Clarity", "Track", "Expired",
                ],
                [[
                    video.bvid.clone(),
                    video.title.chars().take(20).collect(),
                    video.saved.to_string(),
                    video.fav.to_string(),
                    video.upper.name.clone(),
                    video.clarity.unwrap().to_string(),
                    video.track.to_string(),
                    video.expired.to_string(),
                ]],
            );
        }
    }
}

impl ToString for Qn {
    fn to_string(&self) -> String {
        match self {
            Qn::Default => "Default",
            Qn::EightK => "8k",
            Qn::Dolby => "Dolby",
            Qn::HDR => "HDR",
            Qn::FourK => "4k",
            Qn::FullHDHighFrame => "1080P60",
            Qn::FullHDHighCode => "1080P+",
            Qn::FullHD => "1080P",
            Qn::HDHighFrame => "720P60",
            Qn::HD => "720P",
            Qn::SD => "480P",
            Qn::LD => "360P",
            Qn::VLD => "240P",
        }
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::proto::data::Meta;

    #[test]
    fn status_test() {
        Meta::read().status_list(false);
    }

    #[test]
    fn status_of_test() {
        Meta::read().status_of("822720888".to_string());
    }
}
