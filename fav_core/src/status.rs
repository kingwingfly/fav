//! Status of resource

use bitflags::bitflags;

use crate::res::{Res, ResSet};
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
///     fn status(&self) -> i32 {
///         self.status
///     }
///     fn status_mut(&mut self) -> &mut i32 {
///         &mut self.status
///     }
/// }
/// ```
pub trait Status {
    /// return status bits
    fn status(&self) -> i32;
    /// return mutable status bits
    fn status_mut(&mut self) -> &mut i32;
    /// return Statusflags
    fn status_flags(&self) -> StatusFlags {
        StatusFlags::from_bits_retain(self.status())
    }
    /// check whether bits in provided StatusFlags all set
    fn check_status(&self, status: StatusFlags) -> bool {
        self.status_flags().contains(status)
    }
    /// set StatusFlags
    fn set_status(&mut self, status: StatusFlags) {
        *self.status_mut() = status.bits();
    }
    /// set bit of provided StatusFlags
    fn on_status(&mut self, status: StatusFlags) {
        self.set_status(self.status_flags() | status);
    }
    /// unset bit of provided StatusFlags
    fn off_status(&mut self, status: StatusFlags) {
        self.set_status(self.status_flags() - status);
    }
}

bitflags! {
    #[allow(missing_docs)]
    #[derive(Clone, Copy)]
    pub struct StatusFlags: i32 {
        #[allow(missing_docs)]
        const FETCHED = 0b00001;
        #[allow(missing_docs)]
        const TRACK = 0b00010;
        #[allow(missing_docs)]
        const SAVED = 0b00100;
        #[allow(missing_docs)]
        const FAV = 0b01000;
        #[allow(missing_docs)]
        const EXPIRED = 0b10000;
    }
}

/// Status Extension for [`ResSet`]
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::{TestResSet, TestRes};
/// use fav_core::status::{Status, SetStatusExt, StatusFlags};
/// let mut res_set = TestResSet::default();
/// res_set.set.push(TestRes::default());
/// let res_set = res_set.with_res_status_on(StatusFlags::FETCHED);
/// assert!(res_set.set[0].check_status(StatusFlags::FETCHED));
/// ```
pub trait SetStatusExt<R>: ResSet<R>
where
    R: Res,
{
    /// turn on StatusFlags to all resources
    fn on_res_status(&mut self, status: StatusFlags) {
        self.iter_mut().for_each(|r| r.on_status(status));
    }

    /// set StatusFlags to all resources
    fn off_res_status(&mut self, status: StatusFlags) {
        self.iter_mut().for_each(|r| r.off_status(status));
    }

    /// set StatusFlags to all resources
    fn set_res_status(&mut self, status: StatusFlags) {
        self.iter_mut().for_each(|r| r.set_status(status));
    }

    /// turn on StatusFlags to all resources from old
    fn with_res_status_on(mut self, status: StatusFlags) -> Self
    where
        Self: Sized,
    {
        self.iter_mut().for_each(|r| r.on_status(status));
        self
    }

    /// set StatusFlags to all resources from old
    fn with_res_status_set(mut self, status: StatusFlags) -> Self
    where
        Self: Sized,
    {
        self.iter_mut().for_each(|r| r.set_status(status));
        self
    }
}

impl<S, R> SetStatusExt<R> for S
where
    R: Res,
    S: ResSet<R>,
{
}

/// Status Extension for [`Res`]
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::TestRes;
/// use fav_core::status::{Status, ResStatusExt, StatusFlags};
/// let res = TestRes::default().with_status_on(StatusFlags::FETCHED);
/// assert!(res.check_status(StatusFlags::FETCHED));
/// ```
pub trait ResStatusExt: Res + Status + Sized {
    /// turn on StatusFlags to all resources
    fn with_status_on(mut self, status: StatusFlags) -> Self {
        self.on_status(status);
        self
    }

    /// set StatusFlags to all resources
    fn with_status_set(mut self, status: StatusFlags) -> Self {
        self.set_status(status);
        self
    }
}

impl<R> ResStatusExt for R where R: Res + Status + Sized {}

impl From<i32> for StatusFlags {
    fn from(value: i32) -> Self {
        StatusFlags::from_bits_retain(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as fav_core;

    #[derive(Status, Default)]
    struct StatusTest {
        status: i32,
    }

    #[test]
    fn status_flags_test() {
        let mut status = StatusFlags::empty();
        status.insert(StatusFlags::FETCHED);
        assert!(status.intersects(StatusFlags::FETCHED));
        assert_eq!(status.bits(), 1);
        status.insert(StatusFlags::TRACK);
        assert_eq!(status.bits(), 3);
    }

    #[test]
    fn status_test() {
        let mut status = StatusTest::default();
        status.on_status(StatusFlags::FETCHED);
        assert!(status.check_status(StatusFlags::FETCHED));
        status.off_status(StatusFlags::FETCHED);
        assert!(!status.check_status(StatusFlags::FETCHED));
        status.on_status(StatusFlags::FETCHED);
        status.on_status(StatusFlags::SAVED);
        assert!(status.check_status(StatusFlags::FETCHED));
        assert!(status.check_status(StatusFlags::SAVED));
    }
}
