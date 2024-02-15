use crate::proto::bili::{Bili, ResSets};
use fav_core::prelude::*;

impl PathInfo for Bili {
    #[cfg(test)]
    const PATH: &'static str = "temp/temp.bili";
    #[cfg(not(test))]
    const PATH: &'static str = ".fav/bili";
}

impl PathInfo for ResSets {
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

impl Drop for ResSets {
    fn drop(&mut self) {
        self.write();
    }
}
