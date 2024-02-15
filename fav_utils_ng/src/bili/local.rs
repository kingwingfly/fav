use crate::proto::bili::{Bili, BiliSets};
use fav_core::prelude::*;

impl PathInfo for Bili {
    #[cfg(test)]
    const PATH: &'static str = "temp/temp.bili";
    #[cfg(not(test))]
    const PATH: &'static str = ".fav/bili";
}

impl PathInfo for BiliSets {
    #[cfg(test)]
    const PATH: &'static str = "temp/temp.sets";
    #[cfg(not(test))]
    const PATH: &'static str = ".fav/sets";
}

impl Drop for Bili {
    fn drop(&mut self) {
        self.write();
    }
}

impl Drop for BiliSets {
    fn drop(&mut self) {
        self.write();
    }
}
