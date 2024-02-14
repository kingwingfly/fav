//! Status of resource
#![allow(missing_docs)]

use bitflags::bitflags;

/// Making it able to manage the status
pub trait Status {
    /// return &StatusFlags
    fn status(&self) -> &StatusFlags;
    /// return &mut StatusFlags
    fn status_mut(&mut self) -> &mut StatusFlags;
}

bitflags! {
    pub struct StatusFlags: i32 {
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
        let mut status = StatusFlags::empty();
        status.insert(StatusFlags::FETCHED);
        assert!(status.intersects(StatusFlags::FETCHED));
        assert_eq!(status.bits(), 1);
    }
}
