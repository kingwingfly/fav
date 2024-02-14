//! Status of resource
#![allow(missing_docs)]

use bitflags::bitflags;

#[cfg(feature = "derive")]
pub use fav_derive::Status;

/// Making it able to manage the status
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::StatusTest;
/// # use fav_core::status::{StatusFlags, Status};
/// impl Status for StatusTest {
///    fn status(&self) -> StatusFlags {
///        self.status.into()
///    }
///
///    fn set_status(&mut self, status: StatusFlags) {
///        self.status = status.bits();
///    }
/// }
/// ```
pub trait Status {
    /// return &StatusFlags
    fn status(&self) -> StatusFlags;
    /// return &mut StatusFlags
    fn set_status(&mut self, status: StatusFlags);
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

impl From<i32> for StatusFlags {
    fn from(value: i32) -> Self {
        StatusFlags::from_bits_retain(value)
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
