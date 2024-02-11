//! Attribute
//! Contains the traits for resources' attributes.
//! Helping to gain the `id`, `title` and so on for resource, resource set and upper.

#[allow(missing_docs)]
#[derive(Debug)]
pub enum Id {
    I64(i64),
    String(String),
}

impl PartialEq for Id {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Id::I64(a), Id::I64(b)) => a == b,
            (Id::String(a), Id::String(b)) => a == b,
            _ => false,
        }
    }
}

impl From<i64> for Id {
    fn from(value: i64) -> Self {
        Id::I64(value)
    }
}

impl From<String> for Id {
    fn from(value: String) -> Self {
        Id::String(value)
    }
}

/// Attributes
/// #Example
/// ```
/// # use fav_core::attr::{Attr, Id};
///
/// struct Video {
///     id: i64,
///     name: String
/// }
///
/// impl Attr for Video {
///     fn id(&self) -> Id {
///         self.id.into()
///     }
///
///     fn name(&self) -> &str {
///         &self.name
///     }
/// }
///
/// # fn main() {
/// let video = Video {
///     id: 123123,
///     name: "name".to_string()
/// };
///
/// assert_eq!(video.id(), Id::I64(123123));
/// assert_eq!(video.name(), "name");
/// # }
pub trait Attr {
    /// Return the id of the target
    fn id(&self) -> Id; // Todo some id may be usize
    /// Return the name of the target
    fn name(&self) -> &str;
}

/// Attributes of a resource.
pub trait ResAttr: Attr {}

/// Attributes of a resource set.
pub trait ResSetAttr: Attr {}

/// Attributes of a upper.
pub trait UpperAttr: Attr {}
