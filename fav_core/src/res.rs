//! Relations between resources, resource sets, and uppers

use crate::{attr::Attr, meta::Meta};

/// Relations owned by a resource.
pub trait Res: Attr + Send + Sync {
    /// Resource's upper.
    fn uppers(&self) -> impl Iterator<Item = &impl Attr>;

    /// The resource sets that the resource belongs to.
    fn belongs_to(&self) -> impl Iterator<Item = &impl Attr>;

    /// Whether the resource belongs to the resource set.
    fn belongs(&self, resource_set: &impl ResSet) -> bool {
        let id = self.id();
        resource_set.iter().any(|r| r.id() == id)
    }
}

/// Relations owned by a resource set.
pub trait ResSet: Res {
    /// The &resources that the resource set contains.
    fn iter(&self) -> impl Iterator<Item = &impl Meta>;
    /// The &mut resources that the resource set contains.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut impl Meta>;

    /// Whether the resource set contains the resource.
    fn contains(&self, resource: &impl Attr) -> bool {
        let id = resource.id();
        self.iter().any(|r| r.id() == id)
    }
}
