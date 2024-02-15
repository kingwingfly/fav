use crate::proto::bili::Bili;
use fav_core::prelude::*;

impl PathInfo for Bili {
    #[cfg(test)]
    const PATH: &'static str = "temp/temp.bili";
    #[cfg(not(test))]
    const PATH: &'static str = ".fav/bili";
}

impl Drop for Bili {
    fn drop(&mut self) {
        self.write();
    }
}
