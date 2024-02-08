//! Status of resource

/// Status of a resource.
pub trait Status {
    /// Whether the resource has been saved.
    fn saved(&self) -> bool;
    /// Whether the resource is favorite.
    fn fav(&self) -> bool;
    /// Whether the resource is expired.
    fn expired(&self) -> bool;
}
