//! Relations between resources, resource sets, and uppers.

use crate::attr::{ResAttr, ResSetAttr, UpperAttr};

/// Relations owned by a resource.
pub trait ResRel: ResAttr {
    /// Resource's upper.
    fn uppers<U>(&self) -> Option<Vec<U>>
    where
        U: UpperAttr;

    /// The resource sets that the resource belongs to.
    fn belongs_to<RS>(&self) -> Option<Vec<RS>>
    where
        RS: ResSetAttr;

    /// Whether the resource belongs to the resource set.
    fn belongs<R, RS>(&self, resource_set: RS) -> bool
    where
        R: ResAttr,
        RS: ResSetRel<R>,
    {
        let id = self.id();
        resource_set.resources().iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSetRel<R>: ResRel
where
    R: ResAttr,
{
    /// The resources that the resource set contains.
    fn resources(&self) -> Vec<R>;

    /// Whether the resource set contains the resource.
    fn contains(&self, resource: R) -> bool {
        let id = resource.id();
        self.resources().iter().any(|r| r.id() == id)
    }
}

/// Relations owned by an upper.
pub trait UpperRel<R>: ResRel
where
    R: ResAttr,
{
    /// The resources upped by the upper.
    fn resources(&self) -> Vec<R>;

    /// Whether the upper upped the resource.
    fn ups(&self, resouce: R) -> bool {
        let id = resouce.id();
        self.resources().iter().any(|r| r.id() == id)
    }
}
