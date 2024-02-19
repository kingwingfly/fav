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
    fn subset<F>(&mut self, filter: F) -> SubSet<Self, F>
    where
        F: Fn(&dyn Res) -> bool,
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
}

/// A subset of a resource set.
pub struct SubSet<'a, S, F>
where
    S: Set + 'a,
    F: Fn(&dyn Res) -> bool,
{
    set: &'a mut S,
    filter: F,
}

impl<'a, S, F> Set for SubSet<'a, S, F>
where
    S: Set + 'a,
    F: Fn(&dyn Res) -> bool,
{
    type Res = S::Res;

    fn iter(&self) -> impl Iterator<Item = &Self::Res> {
        self.set.iter().filter(|r| (self.filter)(*r))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Res> {
        self.set.iter_mut().filter(|r| (self.filter)(*r))
    }
}
