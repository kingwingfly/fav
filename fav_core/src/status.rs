//! Status of resource

/// Status of a resource.
pub trait Status {
    /// Whether the resource has been fetched
    fn fetched(&self) -> bool;
    /// Whether the resource has been saved.
    fn saved(&self) -> bool;
    /// Whether the resource is favorite.
    fn fav(&self) -> bool;
    /// Whether the resource is expired.
    fn expired(&self) -> bool;

    /// return &mut fetched
    fn fetched_mut(&mut self) -> &mut bool;
    /// return &mut saved
    fn saved_mut(&mut self) -> &mut bool;
    /// return &mut fav
    fn fav_mut(&mut self) -> &mut bool;
    /// return &mut expired
    fn expired_mut(&mut self) -> &mut bool;
}
