use crate::{context::AppContext, error::AppResult};

use super::{
    fetch::{FetchOptions, fetch},
    pull::{PullOptions, pull},
};

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    pub fetch: FetchOptions,
    pub pull: PullOptions,
}

pub async fn sync(ctx: &AppContext, opts: SyncOptions) -> AppResult<()> {
    fetch(ctx, opts.fetch).await?;
    pull(ctx, opts.pull).await
}
