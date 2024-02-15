//! Relations between resources, resource sets, and uppers

use crate::{attr::Attr, meta::Meta};

/// Relations owned by a resource.
pub trait Res: Attr + Send + Sync {
    /// Resource's upper.
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr>;

    /// Returns an iterator over the resource's upper.
    fn uppers_iter(&self) -> impl Iterator<Item = &impl Attr> {
        self.uppers().into_iter()
    }

    /// Whether the resource belongs to the resource set.
    fn belongs(&self, res_set: &impl ResSet) -> bool {
        let id = self.id();
        res_set.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSet: Res {
    /// The &resource that the resource set contains.
    fn res(&self) -> impl IntoIterator<Item = &impl Meta>;
    /// The &mut resource that the resource set contains.
    fn res_mut(&mut self) -> impl IntoIterator<Item = &mut impl Meta>;

    /// The &resource that the resource set contains.
    fn iter(&self) -> impl Iterator<Item = &impl Meta> {
        self.res().into_iter()
    }
    /// The &mut resource that the resource set contains.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut impl Meta> {
        self.res_mut().into_iter()
    }
    /// Whether the set contains the resource.
    fn contains(&self, resource: &impl Attr) -> bool {
        let id = resource.id();
        self.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource sets.
pub trait ResSets {
    /// The &set that the resource sets contains.
    fn sets(&self) -> impl IntoIterator<Item = &impl ResSet>;
    /// The &mut set that the resource sets contains.
    fn sets_mut(&mut self) -> impl IntoIterator<Item = &mut impl ResSet>;

    /// The &set that the resource sets contains.
    fn iter(&self) -> impl Iterator<Item = &impl ResSet> {
        self.sets().into_iter()
    }
    /// The &mut set that the resource sets contains.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut impl ResSet> {
        self.sets_mut().into_iter()
    }
    /// Whether the sets contains the resource set.
    fn contains(&self, set: &impl Attr) -> bool {
        let id = set.id();
        self.iter().any(|r| r.id() == id)
    }
}
