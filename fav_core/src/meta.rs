//! Meta,
//! making resource completely able to be operated

use crate::{attr::Attr, status::Status};

/// Making resource has the complete metadata needed
/// This is an auto trait for `T: Attr + Status`.
/// See [`Attr`] and [`Status`].
pub trait Meta: Attr + Status {}

impl<T: Attr + Status> Meta for T {}
