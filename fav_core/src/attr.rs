//! Attribute,
//! managing the resources' attributes

#[cfg(feature = "derive")]
pub use fav_derive::Attr;

/// The resource's id.
/// # Example
/// ```
/// # use fav_core::attr::Id;
/// let id: Id = 123.into();
/// assert_eq!(id, Id::I32(123));
/// let id: Id = "123".into();
/// assert_eq!(id, Id::I32(123));
/// let id: Id = "68719476735".into();
/// assert_eq!(id, Id::I64(68719476735)); // 68719476735 is 0xFFFF_FFFF_F, which > i32::MAX
/// let id: Id = "abc".into();
/// assert_eq!(id, Id::String("abc"));
/// ```
#[allow(missing_docs)]
#[derive(Debug, PartialEq)]
pub enum Id<'a> {
    I64(i64),
    I32(i32),
    String(&'a str),
}

/// Attributes managing trait.
///
/// You may easily implement the `Attr` trait by deriving [`fav_derive::Attr`].
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::AttrTest;
/// # use fav_core::attr::{Attr, Id};
/// impl Attr for AttrTest {
///     fn id(&self) -> Id {
///        self.id.into()
///     }
///
///     fn title(&self) -> &str {
///         &self.title
///     }
///
///     fn set_id(&mut self, id: Id) {
///         self.id = id.into();
///     }
///
///     fn set_title(&mut self, title: &str) {
///         self.title = title.into()
///     }
/// }
///
/// let res = AttrTest::default();
///
/// assert_eq!(res.id(), 0.into());
/// assert_eq!(res.title(), "");
/// ```
/// Derive macros example:
/// ```
/// # use fav_core::attr::{Attr, Id};
/// #[derive(Attr)]
/// struct AttrTest {
///     id: i32,
///     title: String,
/// }
/// ```
/// More examples in [`fav_derive::Attr`].
#[allow(missing_docs)]
pub trait Attr {
    /// Return the id of the target
    fn id(&self) -> Id;
    /// Return the title of the target
    fn title(&self) -> &str;
    fn set_id(&mut self, id: Id);
    fn set_title(&mut self, title: &str);
}

impl From<i64> for Id<'_> {
    fn from(id: i64) -> Self {
        Id::I64(id)
    }
}

impl From<i32> for Id<'_> {
    fn from(id: i32) -> Self {
        Id::I32(id)
    }
}

impl From<&i64> for Id<'_> {
    fn from(id: &i64) -> Self {
        Id::I64(*id)
    }
}

impl From<&i32> for Id<'_> {
    fn from(id: &i32) -> Self {
        Id::I32(*id)
    }
}

impl<'a> From<&'a str> for Id<'a> {
    fn from(id: &'a str) -> Self {
        match id.parse::<i32>() {
            Ok(id) => Id::I32(id),
            Err(_) => match id.parse::<i64>() {
                Ok(id) => Id::I64(id),
                Err(_) => Id::String(id),
            },
        }
    }
}

impl<'a> From<&'a String> for Id<'a> {
    fn from(id: &'a String) -> Self {
        id.as_str().into()
    }
}

impl From<Id<'_>> for i32 {
    fn from(value: Id) -> Self {
        match value {
            Id::I32(id) => id,
            _ => panic!("Not i32 id"),
        }
    }
}

impl From<Id<'_>> for i64 {
    fn from(value: Id) -> Self {
        match value {
            Id::I64(id) => id,
            _ => panic!("Not i64 id"),
        }
    }
}

impl From<Id<'_>> for String {
    fn from(value: Id) -> Self {
        match value {
            Id::String(id) => id.to_owned(),
            Id::I64(id) => id.to_string(),
            Id::I32(id) => id.to_string(),
        }
    }
}
