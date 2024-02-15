//! Relations between resources, resource sets, and uppers

use crate::{attr::Attr, meta::Meta, prelude::Id};

/// Relations owned by a resource.
pub trait Res: Meta {
    /// Resource's upper.
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr>;

    /// Returns an iterator over the resource's upper.
    fn uppers_iter(&self) -> impl Iterator<Item = &impl Attr> {
        self.uppers().into_iter()
    }

    /// Whether the resource belongs to the resource set.
    fn belongs<'s>(&'s self, res_set: &'s impl ResSet<'s, Self>) -> bool
    where
        Self: Sized,
    {
        let id = self.id();
        res_set.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSet<'s, R: Res + 's>: Res {
    /// The &resource that the resource set contains.
    fn res(&'s self) -> impl IntoIterator<Item = &'s R>;
    /// The &mut resource that the resource set contains.
    fn res_mut(&'s mut self) -> impl IntoIterator<Item = &'s mut R>;
    /// Push a resource to the set.
    fn push(&mut self, resource: R);
    /// Remove a resource from the set.
    fn remove(&mut self, id: Id);

    /// The &resource that the resource set contains.
    fn iter(&'s self) -> impl Iterator<Item = &'s R> {
        self.res().into_iter()
    }
    /// The &mut resource that the resource set contains.
    fn iter_mut(&'s mut self) -> impl Iterator<Item = &'s mut R> {
        self.res_mut().into_iter()
    }
    /// Whether the set contains the resource.
    fn contains(&'s self, resource: &R) -> bool {
        let id = resource.id();
        self.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource sets.
pub trait ResSets<'s, R: Res + 's, S: ResSet<'s, R> + 's> {
    /// The &set that the resource sets contains.
    fn sets(&'s self) -> impl IntoIterator<Item = &'s S>;
    /// The &mut set that the resource sets contains.
    fn sets_mut(&'s mut self) -> impl IntoIterator<Item = &'s mut S>;

    /// The &set that the resource sets contains.
    fn iter(&'s self) -> impl Iterator<Item = &'s S> {
        self.sets().into_iter()
    }
    /// The &mut set that the resource sets contains.
    fn iter_mut(&'s mut self) -> impl Iterator<Item = &'s mut S> {
        self.sets_mut().into_iter()
    }
    /// Whether the sets contains the resource set.
    fn contains(&'s self, set: &S) -> bool {
        let id = set.id();
        self.iter().any(|r| r.id() == id)
    }
}
