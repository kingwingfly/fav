use crate::{context::AppContext, error::AppResult};

use super::{
    fetch::{FetchOptions, fetch},
    pull::{PullOptions, pull, resolve_sound_only},
};

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    pub fetch: FetchOptions,
    pub pull: PullOptions,
}

pub async fn sync(ctx: &AppContext, opts: SyncOptions) -> AppResult<()> {
    resolve_sound_only(ctx, &opts.pull)?;
    fetch(ctx, opts.fetch).await?;
    pull(ctx, opts.pull).await
}
