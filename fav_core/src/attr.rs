//! Attribute
//! Contains the traits for resources' attributes.
//! Helping to gain the `id`, `title` and so on for resource, resource set and upper.

/// Attributes
/// #Example
/// ```
/// # use fav_core::attr::Attr;
///
/// struct Video {
///     id: String,
///     name: String
/// }
///
/// impl Attr for Video {
///     fn id(&self) -> &str {
///         &self.id
///     }
///
///     fn name(&self) -> &str {
///         &self.name
///     }
/// }
///
/// # fn main() {
/// let video = Video {
///     id: "123123".to_string(),
///     name: "name".to_string()
/// };
///
/// assert_eq!(video.id().to_string(), "123123");
/// assert_eq!(video.name(), "name");
/// # }
pub trait Attr {
    /// Return the id of the target
    fn id(&self) -> &str; // Todo some id may be usize
    /// Return the name of the target
    fn name(&self) -> &str;
}

/// Attributes of a resource.
pub trait ResAttr: Attr {}

/// Attributes of a resource set.
pub trait ResSetAttr: Attr {}

/// Attributes of a upper.
pub trait UpperAttr: Attr {}
