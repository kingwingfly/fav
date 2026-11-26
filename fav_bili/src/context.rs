use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::Arc,
};

use anyhow::Result;
use console::Term;
use indicatif::{MultiProgress, ProgressDrawTarget};

use crate::{
    api::{BiliClient, BiliClientConfig},
    config::{Config, Workspace, WorkspaceLock},
    db::Db,
    error::AppResult,
    muxer::{FfmpegMuxer, Muxer},
    table::ColorChoice,
};

pub const DEFAULT_JOBS: usize = 4;
pub const DEFAULT_API_CONCURRENCY: usize = 4;

/// Global CLI runtime flags shared across all commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeOptions {
    pub explicit_dir: Option<PathBuf>,
    pub verbose: u8,
    pub quiet: bool,
    pub color: ColorChoice,
    pub no_progress: bool,
    pub non_interactive: bool,
}

/// Shared execution context injected into CLI actions and integration tests.
pub struct AppContext {
    pub workspace: Workspace,
    pub config: Config,
    pub db: Db,
    pub muxer: Arc<dyn Muxer>,
    pub runtime: RuntimeOptions,
    pub api_config: BiliClientConfig,
    pub multi_progress: Option<MultiProgress>,
    _lock: Option<WorkspaceLock>,
}

impl fmt::Debug for AppContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppContext")
            .field("workspace", &self.workspace)
            .field("config", &self.config)
            .field("runtime", &self.runtime)
            .field("api_config", &self.api_config)
            .field("has_lock", &self._lock.is_some())
            .field("has_multi_progress", &self.multi_progress.is_some())
            .finish()
    }
}

impl AppContext {
    /// Open an existing workspace in read-only mode (no workspace lock, no `Migrator::up`).
    pub async fn open_read_only(mut runtime: RuntimeOptions) -> AppResult<Self> {
        let workspace = Workspace::discover_existing(runtime.explicit_dir.as_deref())?;
        let config = Config::load(Some(&workspace))?;
        apply_config_defaults(&mut runtime, &config)?;
        let db = Db::open_existing(&workspace.db_path(), true).await?;
        let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(config.ffmpeg_path.clone()));
        let multi_progress = build_multi_progress(&runtime);
        Ok(Self {
            workspace,
            config,
            db,
            muxer,
            runtime,
            api_config: BiliClientConfig::default(),
            multi_progress,
            _lock: None,
        })
    }

    /// Open (or create when `create_if_missing` is `true`) a workspace in read-write mode,
    /// acquiring the exclusive `.fav/fav.lock`, enforcing `0700`/`0600` permissions, and running migrations.
    pub async fn open_read_write(
        mut runtime: RuntimeOptions,
        create_if_missing: bool,
    ) -> AppResult<Self> {
        let workspace = if create_if_missing {
            Workspace::discover_or_init(runtime.explicit_dir.as_deref())?
        } else {
            Workspace::discover_existing(runtime.explicit_dir.as_deref())?
        };

        let config = Config::load(Some(&workspace))?;
        apply_config_defaults(&mut runtime, &config)?;

        let lock = workspace.acquire_lock()?;
        workspace.ensure_dirs()?;

        let db = if create_if_missing {
            Db::open_or_create(&workspace.db_path()).await?
        } else {
            Db::open_existing(&workspace.db_path(), false).await?
        };

        let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(config.ffmpeg_path.clone()));
        let multi_progress = build_multi_progress(&runtime);
        Ok(Self {
            workspace,
            config,
            db,
            muxer,
            runtime,
            api_config: BiliClientConfig::default(),
            multi_progress,
            _lock: Some(lock),
        })
    }

    /// Construct an injectable context for unit and integration tests (no lock, no TTY progress).
    pub fn for_test(
        workspace: Workspace,
        config: Config,
        db: Db,
        muxer: Arc<dyn Muxer>,
        api_config: BiliClientConfig,
    ) -> Self {
        Self {
            workspace,
            config,
            db,
            muxer,
            runtime: RuntimeOptions {
                no_progress: true,
                non_interactive: true,
                ..Default::default()
            },
            api_config,
            multi_progress: None,
            _lock: None,
        }
    }

    /// Create an isolated per-account `BiliClient` seeded with `cookies`.
    pub fn client_for_account(&self, cookies: &str) -> Result<BiliClient> {
        BiliClient::with_config(cookies, self.api_config.clone())
    }

    /// Create an anonymous `BiliClient` (empty cookie jar, e.g. QR login).
    pub fn anonymous_client(&self) -> Result<BiliClient> {
        BiliClient::with_config("", self.api_config.clone())
    }

    /// Effective download worker concurrency (`--jobs` -> config `jobs` -> default 4).
    pub fn effective_jobs(&self, override_jobs: Option<usize>) -> usize {
        override_jobs
            .or(self.config.jobs)
            .unwrap_or(DEFAULT_JOBS)
            .max(1)
    }

    /// Effective metadata API concurrency (config `api_concurrency` -> default 4).
    pub fn effective_api_concurrency(&self) -> usize {
        self.config
            .api_concurrency
            .unwrap_or(DEFAULT_API_CONCURRENCY)
            .max(1)
    }

    /// Effective media output directory (`--output` -> config `output_dir` -> workspace root).
    pub fn effective_output_dir(&self, override_out: Option<&Path>) -> PathBuf {
        match override_out {
            Some(path) => path.to_path_buf(),
            None => self.config.resolved_output_dir(&self.workspace),
        }
    }

    /// Effective per-stream bandwidth limit in bytes/sec (`--limit-rate` -> config `limit_rate`).
    pub fn effective_limit_rate(&self, override_rate: Option<u64>) -> Option<u64> {
        override_rate.or_else(|| self.config.parsed_limit_rate().ok().flatten())
    }

    /// Whether interactive progress bars should render to stderr.
    pub fn progress_enabled(&self) -> bool {
        !self.runtime.quiet && !self.runtime.no_progress && Term::stderr().is_term()
    }

    /// Print a concise next-step guidance hint to stderr unless `--quiet` is active.
    pub fn hint(&self, msg: &str) {
        if !self.runtime.quiet {
            eprintln!("Next: {msg}");
        }
    }
}

fn apply_config_defaults(runtime: &mut RuntimeOptions, config: &Config) -> AppResult<()> {
    // Validate config rate limit eagerly so malformed TOML values fail fast with ExitKind::Usage.
    let _ = config.parsed_limit_rate()?;

    if config.no_progress == Some(true) {
        runtime.no_progress = true;
    }
    if runtime.color == ColorChoice::Auto
        && let Some(raw) = &config.color
        && let Ok(parsed) = ColorChoice::from_str(raw)
    {
        runtime.color = parsed;
    }
    Ok(())
}

fn build_multi_progress(runtime: &RuntimeOptions) -> Option<MultiProgress> {
    if !runtime.quiet && !runtime.no_progress && Term::stderr().is_term() {
        Some(MultiProgress::with_draw_target(ProgressDrawTarget::stderr()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::muxer::test_support::MockMuxer;

    #[tokio::test]
    async fn open_read_write_locks_while_read_only_stays_concurrent() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = RuntimeOptions {
            explicit_dir: Some(tmp.path().to_path_buf()),
            no_progress: true,
            non_interactive: true,
            ..Default::default()
        };

        let rw = AppContext::open_read_write(runtime.clone(), true)
            .await
            .unwrap();
        assert!(rw.workspace.exists());
        assert_eq!(rw.effective_jobs(None), DEFAULT_JOBS);
        assert_eq!(rw.effective_jobs(Some(8)), 8);
        assert_eq!(rw.effective_api_concurrency(), DEFAULT_API_CONCURRENCY);
        assert_eq!(rw.effective_output_dir(None), tmp.path());

        // Second read-write open in same workspace must fail due to WorkspaceLock
        assert!(
            AppContext::open_read_write(runtime.clone(), false)
                .await
                .is_err()
        );

        // Read-only open does not take lock and succeeds concurrently
        let ro = AppContext::open_read_only(runtime.clone()).await.unwrap();
        assert!(ro.db.all_accounts().await.unwrap().is_empty());

        drop(rw);
        // After drop, lock is released
        let _rw2 = AppContext::open_read_write(runtime, false).await.unwrap();
    }

    #[tokio::test]
    async fn for_test_respects_config_overrides_and_builds_clients() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        let cfg = Config {
            output_dir: Some(PathBuf::from("downloads")),
            jobs: Some(2),
            limit_rate: Some("2M".to_owned()),
            api_concurrency: Some(6),
            ..Default::default()
        };
        let db = Db::open_in_memory().await.unwrap();
        let muxer: Arc<dyn Muxer> = Arc::new(MockMuxer::succeeding());
        let ctx = AppContext::for_test(ws, cfg, db, muxer, BiliClientConfig::default());

        assert_eq!(ctx.effective_jobs(None), 2);
        assert_eq!(ctx.effective_jobs(Some(0)), 1);
        assert_eq!(ctx.effective_api_concurrency(), 6);
        assert_eq!(ctx.effective_output_dir(None), tmp.path().join("downloads"));
        assert_eq!(
            ctx.effective_output_dir(Some(Path::new("/tmp/custom"))),
            PathBuf::from("/tmp/custom")
        );
        assert_eq!(ctx.effective_limit_rate(None), Some(2 * 1024 * 1024));
        assert_eq!(ctx.effective_limit_rate(Some(500)), Some(500));
        assert!(!ctx.progress_enabled());
        assert!(ctx.client_for_account("SESSDATA=abc").is_ok());
        assert!(ctx.anonymous_client().is_ok());
    }
}
