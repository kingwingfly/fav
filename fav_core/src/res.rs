//! Relations between resources, resource sets, and uppers

use crate::{attr::Attr, attr::Id, meta::Meta};

/// Relations owned by a resource.
pub trait Res: Meta {
    /// Resource upper.
    fn upper(&self) -> &impl Attr;

    /// Whether the resource belongs to the resource set.
    fn belongs(&self, res_set: &impl ResSet<Self>) -> bool
    where
        Self: Sized,
    {
        let id = self.id();
        res_set.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSet<R: Res>: Res {
    /// The &resource that the resource set contains.
    fn res<'a>(&'a self) -> impl IntoIterator<Item = &'a R>
    where
        R: 'a;
    /// The &mut resource that the resource set contains.
    fn res_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut R>
    where
        R: 'a;
    /// Push a resource to the set.
    fn push(&mut self, resource: R);
    /// Remove a resource from the set.
    fn remove(&mut self, id: Id);

    /// The &resource that the resource set contains.
    fn iter<'a>(&'a self) -> impl Iterator<Item = &'a R>
    where
        R: 'a,
    {
        self.res().into_iter()
    }
    /// The &mut resource that the resource set contains.
    fn iter_mut<'a>(&'a mut self) -> impl Iterator<Item = &'a mut R>
    where
        R: 'a,
    {
        self.res_mut().into_iter()
    }
    /// Whether the set contains the resource.
    fn contains(&self, resource: &R) -> bool {
        let id = resource.id();
        self.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource sets.
pub trait ResSets<S, R>
where
    S: ResSet<R>,
    R: Res,
{
    /// The &set that the resource sets contains.
    fn sets<'a>(&'a self) -> impl IntoIterator<Item = &'a S>
    where
        S: 'a;
    /// The &mut set that the resource sets contains.
    fn sets_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut S>
    where
        S: 'a;

    /// The &set that the resource sets contains.
    fn iter<'a>(&'a self) -> impl Iterator<Item = &'a S>
    where
        S: 'a,
    {
        self.sets().into_iter()
    }
    /// The &mut set that the resource sets contains.
    fn iter_mut<'a>(&'a mut self) -> impl Iterator<Item = &'a mut S>
    where
        S: 'a,
    {
        self.sets_mut().into_iter()
    }
    /// Whether the sets contains the resource set.
    fn contains(&self, set: &S) -> bool {
        let id = set.id();
        self.iter().any(|r| r.id() == id)
    }
}
