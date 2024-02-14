//! Status of resource
#![allow(missing_docs)]

use bitflags::bitflags;

bitflags! {
    pub struct Status: u8 {
        const FETCHED = 0b00001;
        const TRACK = 0b00010;
        const SAVED = 0b00100;
        const FAV = 0b01000;
        const EXPIRED = 0b10000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_test() {
        let mut status = Status::empty();
        status.insert(Status::FETCHED);
        assert!(status.intersects(Status::FETCHED));
    }
}
