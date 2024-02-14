//! Relations between resources, resource sets, and uppers.

use crate::attr::{Attr, ResAttr, ResSetAttr};

/// Relations owned by a resource.
pub trait ResRel: ResAttr + Send + Sync {
    /// Resource's upper.
    fn uppers(&self) -> Option<Vec<&impl ResSetAttr>>;

    /// The resource sets that the resource belongs to.
    fn belongs_to(&self) -> Option<Vec<&impl ResSetAttr>>;

    /// Whether the resource belongs to the resource set.
    fn belongs(&self, resource_set: &impl ResSetRel) -> bool {
        let id = self.id();
        resource_set.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSetRel: ResRel {
    /// The &resources that the resource set contains.
    fn iter(&self) -> impl Iterator<Item = &impl ResAttr>;
    /// The &mut resources that the resource set contains.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut impl ResAttr>;

    /// Whether the resource set contains the resource.
    fn contains(&self, resource: &impl Attr) -> bool {
        let id = resource.id();
        self.iter().any(|r| r.id() == id)
    }
}
