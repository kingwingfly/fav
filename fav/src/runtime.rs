use std::ops::Deref;

use anyhow::Result;
use bevy_ecs::resource::Resource;
use tokio::runtime::Builder;

#[derive(Debug, Resource)]
pub struct Runtime(tokio::runtime::Runtime);

impl Deref for Runtime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Runtime {
    pub fn new() -> Result<Self> {
        Builder::new_current_thread()
            .enable_io()
            .build()
            .map(Runtime)
            .map_err(Into::into)
    }
}
