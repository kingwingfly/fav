//! Relations between resources, resource sets, and uppers.

use crate::attr::{ResAttr, ResSetAttr};
use std::slice::{Iter, IterMut};

/// Relations owned by a resource.
pub trait ResRel: ResAttr + Send + Sync {
    /// Resource's upper.
    fn uppers<RS>(&self) -> Option<Vec<&RS>>
    where
        RS: ResSetAttr;

    /// The resource sets that the resource belongs to.
    fn belongs_to<RS>(&self) -> Option<Vec<&RS>>
    where
        RS: ResSetAttr;

    /// Whether the resource belongs to the resource set.
    fn belongs<R, RS>(&self, resource_set: &RS) -> bool
    where
        R: ResRel,
        RS: ResSetRel<R>,
    {
        let id = self.id();
        resource_set.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSetRel<R>: ResRel
where
    R: ResRel,
{
    /// The resources that the resource set contains.
    fn iter(&self) -> Iter<R>;
    /// The &mut resources that the resource set contains.
    fn iter_mut(&mut self) -> IterMut<R>;

    /// Whether the resource set contains the resource.
    fn contains(&self, resource: &R) -> bool {
        let id = resource.id();
        self.iter().any(|r| r.id() == id)
    }
}
