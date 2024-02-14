//! Attribute
//! Contains the traits for resources' attributes.
//! Helping to gain the `id`, `title` and so on for resource, resource set and upper.

use std::str::FromStr;

#[allow(missing_docs)]
#[derive(Debug, PartialEq)]
pub enum Id {
    I64(i64),
    String(String),
}

impl From<i64> for Id {
    fn from(id: i64) -> Self {
        Id::I64(id)
    }
}

impl FromStr for Id {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<i64>() {
            Ok(id) => Ok(Id::I64(id)),
            Err(_) => Ok(Id::String(s.to_owned())),
        }
    }
}

impl From<Id> for String {
    fn from(value: Id) -> Self {
        match value {
            Id::I64(id) => id.to_string(),
            Id::String(id) => id,
        }
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
///        Id::I64(self.id)
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
/// assert_eq!(video.id(), 123123.into());
/// assert_eq!(video.name(), "name");
/// # }
pub trait Attr: Send + Sync {
    /// Return the id of the target
    fn id(&self) -> Id;
    /// Return the name of the target
    fn name(&self) -> &str;
}

/// Attributes of a resource.
pub trait ResAttr: Attr {}

/// Attributes of a resource set.
pub trait ResSetAttr: Attr {}
