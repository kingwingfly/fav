//! Attribute
//! Contains the traits for resources's attributes.
//! Helping to gain the `id`, `title` and so on for resource, resource set and upper.

/// Attributes of a resource.
pub trait ResAttr {
    /// Resource id
    fn id(&self) -> &str;
    /// Resouce name
    fn name(&self) -> &str;
}

/// Attributes of a resource set.
pub trait ResSetAttr {
    /// Resource set id
    fn id(&self) -> &str;
    /// Resource set name
    fn name(&self) -> &str;
}

/// Attributes of a upper.
pub trait UpperAttr {
    /// Upper id
    fn id(&self) -> &str;
    /// Upper name
    fn name(&self) -> &str;
}
