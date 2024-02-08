//! Attribute
//! Contains the traits for resources' attributes.
//! Helping to gain the `id`, `title` and so on for resource, resource set and upper.

/// Attributes
pub trait Attr {
    /// id
    fn id(&self) -> &str;
    /// name
    fn name(&self) -> &str;
}

/// Attributes of a resource.
pub trait ResAttr: Attr {}

/// Attributes of a resource set.
pub trait ResSetAttr: Attr {}

/// Attributes of a upper.
pub trait UpperAttr: Attr {}
