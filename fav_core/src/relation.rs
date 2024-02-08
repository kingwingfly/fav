//! Relations between resources, resource sets, and uppers.

use crate::attr::{ResAttr, ResSetAttr};

/// Relations owned by a resource.
pub trait ResRel: ResAttr {
    /// Resource's upper.
    fn upper<US>(&self) -> Option<US>
    where
        US: IntoIterator,
        US::Item: ResAttr;

    /// The resource sets that the resource belongs to.
    fn belongs_to<RS>(&self) -> Option<RS>
    where
        RS: IntoIterator,
        RS::Item: ResSetAttr;

    /// Whether the resource belongs to the resource set.
    fn belongs<R, RS>(&self, resource_set: RS) -> bool
    where
        R: ResAttr,
        RS: ResSetRel,
    {
        let id = self.id();
        resource_set
            .resources::<R, Vec<R>>()
            .iter()
            .any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSetRel {
    /// The resources that the resource set contains.
    fn resources<R, RS>(&self) -> RS
    where
        R: ResAttr,
        RS: IntoIterator<Item = R>;

    /// Whether the resource set contains the resource.
    fn contains<R>(&self, resource: R) -> bool
    where
        R: ResAttr,
    {
        let id = resource.id();
        self.resources::<R, Vec<R>>().iter().any(|r| r.id() == id)
    }
}

/// Relations owned by an upper.
pub trait UpperRel {
    /// The resources upped by the upper.
    fn resources<R, RS>(&self) -> RS
    where
        R: ResAttr,
        RS: IntoIterator<Item = R>;

    /// Whether the upper upped the resource.
    fn ups<R>(&self, resouce: R) -> bool
    where
        R: ResAttr;
}
