//! Relations between resources, resource sets, and uppers

use crate::meta::Meta;

/// Relations owned by a resource.
pub trait Res: Meta {}

/// Relations owned by a resource set.
pub trait Set {
    /// The resource type in this set
    type Res: Res;
    /// The &resource that the resource set contains.
    fn iter(&self) -> impl Iterator<Item = &Self::Res>;
    /// The &mut resource that the resource set contains.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Res>;

    /// Get a subset of the resource set.
    /// # Example
    /// ```no_run
    /// # #[path = "test_utils/mod.rs"]
    /// # mod test_utils;
    /// # use test_utils::data::{App, TestSet};
    /// # use fav_core::{status::{Status, StatusFlags}, res::Set, ops::ResOpsExt};
    /// # async {
    /// let app = App::default();
    /// let mut set = TestSet::default();
    /// let mut sub = set.subset(|r| r.check_status(StatusFlags::TRACK));
    /// app.batch_fetch_res(&mut sub);
    /// # };
    /// ```
    fn subset<F>(&mut self, filter: F) -> SubSet<Self, F>
    where
        F: Fn(&Self::Res) -> bool,
        Self: Sized,
    {
        SubSet { set: self, filter }
    }
}

/// Relations owned by a resource sets.
pub trait Sets {
    /// The resource set type in this sets
    type Set: Set;
    /// The &set that the resource sets contains.
    fn iter(&self) -> impl Iterator<Item = &Self::Set>;
    /// The &mut set that the resource sets contains.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Set>;

    /// Get a subset of the resource sets.
    /// # Example
    /// ```no_run
    /// # #[path = "test_utils/mod.rs"]
    /// # mod test_utils;
    /// # use test_utils::data::{App, TestSets};
    /// # use fav_core::{status::{Status, StatusFlags}, res::Sets, ops::SetOpsExt};
    /// # async {
    /// let app = App::default();
    /// let mut sets = TestSets::default();
    /// let mut sub = sets.subset(|r| r.check_status(StatusFlags::TRACK));
    /// app.batch_fetch_set(&mut sub);
    /// # };
    /// ```
    fn subset<F>(&mut self, filter: F) -> SubSets<Self, F>
    where
        F: Fn(&Self::Set) -> bool,
        Self: Sized,
    {
        SubSets { sets: self, filter }
    }
}

/// A subset of a resource set.
pub struct SubSet<'a, S, F>
where
    S: Set + 'a,
    F: Fn(&S::Res) -> bool,
{
    set: &'a mut S,
    filter: F,
}

impl<'a, S, F> Set for SubSet<'a, S, F>
where
    S: Set + 'a,
    F: Fn(&S::Res) -> bool,
{
    type Res = S::Res;

    fn iter(&self) -> impl Iterator<Item = &Self::Res> {
        self.set.iter().filter(|r| (self.filter)(*r))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Res> {
        self.set.iter_mut().filter(|r| (self.filter)(*r))
    }
}

/// A subset of a resource sets.
pub struct SubSets<'a, SS, F>
where
    SS: Sets + 'a,
    F: Fn(&SS::Set) -> bool,
{
    sets: &'a mut SS,
    filter: F,
}

impl<'a, SS, F> Sets for SubSets<'a, SS, F>
where
    SS: Sets + 'a,
    F: Fn(&SS::Set) -> bool,
{
    type Set = SS::Set;

    fn iter(&self) -> impl Iterator<Item = &Self::Set> {
        self.sets.iter().filter(|s| (self.filter)(*s))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Set> {
        self.sets.iter_mut().filter(|s| (self.filter)(*s))
    }
}
