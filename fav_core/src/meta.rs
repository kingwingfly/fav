//! Meta,
//! making resource completely able to be operated

use crate::{attr::Attr, status::Status};

/// Making resource has the complete metadata needed
pub trait Meta: Attr + Status + Send + Sync {}

impl<T: Attr + Status + Send + Sync> Meta for T {}
