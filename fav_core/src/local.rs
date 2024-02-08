//! Local

/// Local utils to help read and write
pub trait Local {
    /// The path to the local
    fn path(&self) -> &str;
    /// Save the resource
    fn save();
    /// Remove the resource
    fn remove();
}
