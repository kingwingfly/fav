use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    str::FromStr as _,
};

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    state::{AudioFormat, VideoClarity, VideoCodec},
};

pub const FAV_DIR_ENV: &str = "FAV_DIR";
pub const FAV_META_DIR: &str = ".fav";
pub const FAV_DB_FILE: &str = "fav.db";
pub const FAV_LOCK_FILE: &str = "fav.lock";
pub const FAV_TMP_DIR: &str = "tmp";
pub const FAV_CONFIG_FILE: &str = "config.toml";

/// Return the user's configuration root directory (e.g. `~/.config` on Unix, `%APPDATA%` on Windows).
pub fn user_config_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join("AppData").join("Roaming"))
            })
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            })
    }
}

/// Return the user's default data directory for `fav` (e.g. `~/.local/share/fav`).
pub fn user_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("fav"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("fav"))
            .or_else(|| {
                std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| {
                    PathBuf::from(h)
                        .join("Library")
                        .join("Application Support")
                        .join("fav")
                })
            })
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("fav"))
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".local").join("share").join("fav"))
            })
    }
}

/// Path to the global user config file (`~/.config/fav/config.toml`).
pub fn global_config_path() -> Option<PathBuf> {
    user_config_root().map(|dir| dir.join("fav").join(FAV_CONFIG_FILE))
}

/// A `fav` workspace root containing `.fav/fav.db`, `.fav/config.toml`, `.fav/tmp`, and `.fav/fav.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn fav_dir(&self) -> PathBuf {
        self.root.join(FAV_META_DIR)
    }

    pub fn db_path(&self) -> PathBuf {
        self.fav_dir().join(FAV_DB_FILE)
    }

    pub fn lock_path(&self) -> PathBuf {
        self.fav_dir().join(FAV_LOCK_FILE)
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.fav_dir().join(FAV_TMP_DIR)
    }

    pub fn config_path(&self) -> PathBuf {
        self.fav_dir().join(FAV_CONFIG_FILE)
    }

    pub fn exists(&self) -> bool {
        self.db_path().is_file()
    }

    /// Create `.fav/` and `.fav/tmp/` and apply private `0700` permissions on Unix.
    pub fn ensure_dirs(&self) -> Result<()> {
        let fav_dir = self.fav_dir();
        std::fs::create_dir_all(&fav_dir).with_context(|| {
            format!(
                "Failed to create workspace metadata dir '{}'",
                fav_dir.display()
            )
        })?;
        set_dir_permissions_private(&fav_dir)?;

        let tmp_dir = self.tmp_dir();
        std::fs::create_dir_all(&tmp_dir).with_context(|| {
            format!("Failed to create temp staging dir '{}'", tmp_dir.display())
        })?;
        set_dir_permissions_private(&tmp_dir)?;
        Ok(())
    }

    /// Acquire an exclusive non-blocking process lock on `.fav/fav.lock`.
    pub fn acquire_lock(&self) -> Result<WorkspaceLock> {
        WorkspaceLock::acquire(self)
    }

    /// Discover an existing workspace for read/sync commands.
    /// Precedence: `-C/--dir` -> `FAV_DIR` -> upward walk from CWD -> default XDG data dir.
    pub fn discover_existing(explicit_dir: Option<&Path>) -> Result<Self> {
        let env_dir = std::env::var_os(FAV_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let cwd =
            std::env::current_dir().context("Failed to determine current working directory")?;
        let fallback = user_data_dir();
        Self::discover_existing_from(explicit_dir, env_dir.as_deref(), &cwd, fallback.as_deref())
    }

    /// Discover an existing workspace or pick a target root for `init` / `auth login` / `auth usecookies`.
    /// Precedence: `-C/--dir` -> `FAV_DIR` -> upward walk from CWD (if `.fav/fav.db` exists) -> CWD.
    pub fn discover_or_init(explicit_dir: Option<&Path>) -> Result<Self> {
        let env_dir = std::env::var_os(FAV_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let cwd =
            std::env::current_dir().context("Failed to determine current working directory")?;
        Self::discover_or_init_from(explicit_dir, env_dir.as_deref(), &cwd)
    }

    /// Alias for [`Workspace::discover_or_init`] when callers want a single entry point.
    pub fn discover(explicit_dir: Option<&Path>) -> Result<Self> {
        Self::discover_or_init(explicit_dir)
    }

    pub fn discover_existing_from(
        explicit_dir: Option<&Path>,
        env_dir: Option<&Path>,
        start_dir: &Path,
        fallback_dir: Option<&Path>,
    ) -> Result<Self> {
        if let Some(dir) = explicit_dir {
            let ws = Self::new(dir.to_path_buf());
            if ws.exists() {
                return Ok(ws);
            }
            return Err(AppError::usage(anyhow!(
                "No fav database found in '{}' (expected '{}'). Run `fav -C {} init` or `fav -C {} auth login` first.",
                dir.display(),
                ws.db_path().display(),
                dir.display(),
                dir.display(),
            ))
            .into());
        }

        if let Some(dir) = env_dir {
            let ws = Self::new(dir.to_path_buf());
            if ws.exists() {
                return Ok(ws);
            }
            return Err(AppError::usage(anyhow!(
                "FAV_DIR='{}' does not contain '{}'. Run `fav init` or `fav auth login` there, or unset FAV_DIR.",
                dir.display(),
                ws.db_path().display(),
            ))
            .into());
        }

        if let Some(ws) = find_upward(start_dir) {
            return Ok(ws);
        }

        if let Some(dir) = fallback_dir {
            let ws = Self::new(dir.to_path_buf());
            if ws.exists() {
                return Ok(ws);
            }
        }

        Err(AppError::usage(anyhow!(
            "No fav workspace found (searched upward from '{}' and default data dir). Run `fav init` or `fav auth login` first, or pass `-C/--dir <PATH>`.",
            start_dir.display(),
        ))
        .into())
    }

    pub fn discover_or_init_from(
        explicit_dir: Option<&Path>,
        env_dir: Option<&Path>,
        start_dir: &Path,
    ) -> Result<Self> {
        if let Some(dir) = explicit_dir {
            return Ok(Self::new(dir.to_path_buf()));
        }
        if let Some(dir) = env_dir {
            return Ok(Self::new(dir.to_path_buf()));
        }
        if let Some(ws) = find_upward(start_dir) {
            return Ok(ws);
        }
        Ok(Self::new(start_dir.to_path_buf()))
    }
}

fn find_upward(start_dir: &Path) -> Option<Workspace> {
    let mut current = Some(start_dir);
    while let Some(dir) = current {
        let candidate = Workspace::new(dir.to_path_buf());
        if candidate.exists() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

/// Persistent user/workspace configuration loaded from TOML files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub output_dir: Option<PathBuf>,
    pub jobs: Option<usize>,
    pub limit_rate: Option<String>,
    pub prune: Option<bool>,
    pub no_progress: Option<bool>,
    pub color: Option<String>,
    pub ffmpeg_path: Option<PathBuf>,
    pub api_concurrency: Option<usize>,
    #[serde(alias = "quality")]
    pub clarity: Option<String>,
    pub codec: Option<String>,
    pub sound_only: Option<bool>,
    pub audio_format: Option<String>,
    pub recheck: Option<bool>,
    pub tmp_dir: Option<PathBuf>,
}

impl Config {
    /// Load global (`~/.config/fav/config.toml`) and workspace (`<ws>/.fav/config.toml`) configs,
    /// with workspace keys overriding global keys.
    pub fn load(workspace: Option<&Workspace>) -> Result<Self> {
        let global = global_config_path();
        let local = workspace.map(Workspace::config_path);
        Self::load_from_paths(global.as_deref(), local.as_deref())
    }

    pub fn load_from_paths(global: Option<&Path>, local: Option<&Path>) -> Result<Self> {
        let mut cfg = Self::default();
        if let Some(path) = global
            && let Some(loaded) = Self::load_file(path)?
        {
            cfg.merge(loaded);
        }
        if let Some(path) = local
            && let Some(loaded) = Self::load_file(path)?
        {
            cfg.merge(loaded);
        }
        Ok(cfg)
    }

    pub fn load_file(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file '{}'", path.display()))?;
        let parsed: Self = toml::from_str(&content).map_err(|err| {
            AppError::usage(anyhow::Error::new(err).context(format!(
                "Failed to parse TOML config file '{}'",
                path.display()
            )))
        })?;
        Ok(Some(parsed))
    }

    pub fn merge(&mut self, overlay: Self) {
        if overlay.output_dir.is_some() {
            self.output_dir = overlay.output_dir;
        }
        if overlay.jobs.is_some() {
            self.jobs = overlay.jobs;
        }
        if overlay.limit_rate.is_some() {
            self.limit_rate = overlay.limit_rate;
        }
        if overlay.prune.is_some() {
            self.prune = overlay.prune;
        }
        if overlay.no_progress.is_some() {
            self.no_progress = overlay.no_progress;
        }
        if overlay.color.is_some() {
            self.color = overlay.color;
        }
        if overlay.ffmpeg_path.is_some() {
            self.ffmpeg_path = overlay.ffmpeg_path;
        }
        if overlay.api_concurrency.is_some() {
            self.api_concurrency = overlay.api_concurrency;
        }
        if overlay.clarity.is_some() {
            self.clarity = overlay.clarity;
        }
        if overlay.codec.is_some() {
            self.codec = overlay.codec;
        }
        if overlay.sound_only.is_some() {
            self.sound_only = overlay.sound_only;
        }
        if overlay.audio_format.is_some() {
            self.audio_format = overlay.audio_format;
        }
        if overlay.recheck.is_some() {
            self.recheck = overlay.recheck;
        }
        if overlay.tmp_dir.is_some() {
            self.tmp_dir = overlay.tmp_dir;
        }
    }

    /// Resolve effective output directory relative to `workspace.root()` when relative.
    pub fn resolved_output_dir(&self, workspace: &Workspace) -> PathBuf {
        match &self.output_dir {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => workspace.root().join(p),
            None => workspace.root().to_path_buf(),
        }
    }

    /// Resolve effective staging temp directory relative to `workspace.root()` when relative,
    /// falling back to `workspace.tmp_dir()` (`.fav/tmp`).
    pub fn resolved_tmp_dir(&self, workspace: &Workspace) -> PathBuf {
        match &self.tmp_dir {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => workspace.root().join(p),
            None => workspace.tmp_dir(),
        }
    }

    /// Parse configured `limit_rate` into bytes per second, if set.
    pub fn parsed_limit_rate(&self) -> Result<Option<u64>> {
        self.limit_rate.as_deref().map(parse_rate_limit).transpose()
    }

    /// Parse configured `clarity` (or alias `quality`), if set.
    pub fn parsed_clarity(&self) -> Result<Option<VideoClarity>> {
        self.clarity
            .as_deref()
            .map(|s| {
                VideoClarity::from_str(s)
                    .map_err(|msg| anyhow::Error::from(AppError::usage(anyhow!("{msg}"))))
            })
            .transpose()
    }

    /// Parse configured `codec`, if set.
    pub fn parsed_codec(&self) -> Result<Option<VideoCodec>> {
        self.codec
            .as_deref()
            .map(|s| {
                VideoCodec::from_str(s)
                    .map_err(|msg| anyhow::Error::from(AppError::usage(anyhow!("{msg}"))))
            })
            .transpose()
    }

    /// Parse configured `audio_format`, if set.
    pub fn parsed_audio_format(&self) -> Result<Option<AudioFormat>> {
        self.audio_format
            .as_deref()
            .map(|s| {
                AudioFormat::from_str(s)
                    .map_err(|msg| anyhow::Error::from(AppError::usage(anyhow!("{msg}"))))
            })
            .transpose()
    }
}

/// Parse human-readable rate limits such as `"5M"`, `"500K"`, `"2.5MB"`, or raw bytes/sec `"1048576"`.
pub fn parse_rate_limit(input: &str) -> Result<u64> {
    let s = input.trim();
    if s.is_empty() {
        return Err(AppError::usage(anyhow!("Rate limit must not be empty")).into());
    }
    let s = s
        .strip_suffix("/s")
        .or_else(|| s.strip_suffix("/S"))
        .unwrap_or(s);
    let upper = s.to_ascii_uppercase();
    let (num_str, mult): (&str, f64) = if let Some(n) = upper
        .strip_suffix("GIB")
        .or_else(|| upper.strip_suffix("GB"))
        .or_else(|| upper.strip_suffix('G'))
    {
        (n, 1024.0 * 1024.0 * 1024.0)
    } else if let Some(n) = upper
        .strip_suffix("MIB")
        .or_else(|| upper.strip_suffix("MB"))
        .or_else(|| upper.strip_suffix('M'))
    {
        (n, 1024.0 * 1024.0)
    } else if let Some(n) = upper
        .strip_suffix("KIB")
        .or_else(|| upper.strip_suffix("KB"))
        .or_else(|| upper.strip_suffix('K'))
    {
        (n, 1024.0)
    } else if let Some(n) = upper.strip_suffix('B') {
        (n, 1.0)
    } else {
        (upper.as_str(), 1.0)
    };

    let value: f64 = num_str.trim().parse().map_err(|_| {
        AppError::usage(anyhow!(
            "Invalid rate limit '{input}' (expected e.g. 5M, 500K, 1048576)"
        ))
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(AppError::usage(anyhow!("Rate limit '{input}' must be positive")).into());
    }
    let bytes = (value * mult).round();
    if !bytes.is_finite() || bytes < 1.0 || bytes > u64::MAX as f64 {
        return Err(AppError::usage(anyhow!("Rate limit '{input}' is out of range")).into());
    }
    Ok(bytes as u64)
}

/// Exclusive process lock guarding a workspace (`.fav/fav.lock`) with RAII unlock on [`Drop`].
#[derive(Debug)]
pub struct WorkspaceLock {
    file: File,
    path: PathBuf,
}

impl WorkspaceLock {
    pub fn acquire(workspace: &Workspace) -> Result<Self> {
        let fav_dir = workspace.fav_dir();
        std::fs::create_dir_all(&fav_dir)
            .with_context(|| format!("Failed to create '{}'", fav_dir.display()))?;
        set_dir_permissions_private(&fav_dir)?;

        let path = workspace.lock_path();
        let file = open_private_file(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file, path }),
            Err(std::fs::TryLockError::WouldBlock) => Err(AppError::runtime(anyhow!(
                "Another `fav` process is already running in workspace '{}' (lock held at '{}'). Wait for it to finish before retrying.",
                workspace.root().display(),
                path.display(),
            ))
            .into()),
            Err(std::fs::TryLockError::Error(e)) => Err(e)
                .with_context(|| format!("Failed to lock workspace file '{}'", path.display())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Open or create a file with `0600` permissions on Unix.
pub fn open_private_file(path: &Path) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let file = opts
        .open(path)
        .with_context(|| format!("Failed to open '{}'", path.display()))?;
    set_file_permissions_private(path)?;
    Ok(file)
}

/// Restrict directory permissions to owner-only (`0700`) on Unix.
pub fn set_dir_permissions_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).with_context(
                || format!("Failed to set 0700 permissions on '{}'", path.display()),
            )?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Restrict file permissions to owner-only (`0600`) on Unix.
pub fn set_file_permissions_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).with_context(
                || format!("Failed to set 0600 permissions on '{}'", path.display()),
            )?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Enforce private permissions on `.fav/`, `fav.db`, `fav.db-wal`, `fav.db-shm`, and `.fav/tmp/`.
pub fn enforce_db_permissions(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        set_dir_permissions_private(parent)?;
        let tmp = parent.join(FAV_TMP_DIR);
        if tmp.exists() {
            set_dir_permissions_private(&tmp)?;
        }
    }
    set_file_permissions_private(db_path)?;

    let mut wal = db_path.as_os_str().to_os_string();
    wal.push("-wal");
    set_file_permissions_private(Path::new(&wal))?;

    let mut shm = db_path.as_os_str().to_os_string();
    shm.push("-shm");
    set_file_permissions_private(Path::new(&shm))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_discovery_precedence_and_upward_walk() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("archive");
        let nested = root.join("sub").join("deep");
        let explicit = tmp.path().join("explicit");
        let env_dir = tmp.path().join("env");
        let fallback = tmp.path().join("xdg");
        std::fs::create_dir_all(&nested).unwrap();

        // Create db in root and fallback
        std::fs::create_dir_all(root.join(".fav")).unwrap();
        std::fs::write(root.join(".fav/fav.db"), b"").unwrap();
        std::fs::create_dir_all(fallback.join(".fav")).unwrap();
        std::fs::write(fallback.join(".fav/fav.db"), b"").unwrap();

        // Upward walk finds root from nested dir
        let ws = Workspace::discover_existing_from(None, None, &nested, Some(&fallback)).unwrap();
        assert_eq!(ws.root, root);

        // Fallback used when outside any workspace
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let ws = Workspace::discover_existing_from(None, None, &outside, Some(&fallback)).unwrap();
        assert_eq!(ws.root, fallback);

        // Explicit dir wins if it exists, errors clearly if missing
        assert!(
            Workspace::discover_existing_from(Some(&explicit), None, &nested, Some(&fallback))
                .is_err()
        );
        std::fs::create_dir_all(explicit.join(".fav")).unwrap();
        std::fs::write(explicit.join(".fav/fav.db"), b"").unwrap();
        let ws = Workspace::discover_existing_from(
            Some(&explicit),
            Some(&env_dir),
            &nested,
            Some(&fallback),
        )
        .unwrap();
        assert_eq!(ws.root, explicit);

        // ENV dir wins over upward walk
        std::fs::create_dir_all(env_dir.join(".fav")).unwrap();
        std::fs::write(env_dir.join(".fav/fav.db"), b"").unwrap();
        let ws = Workspace::discover_existing_from(None, Some(&env_dir), &nested, Some(&fallback))
            .unwrap();
        assert_eq!(ws.root, env_dir);

        // discover_or_init falls back to start_dir when nothing exists
        let ws = Workspace::discover_or_init_from(None, None, &outside).unwrap();
        assert_eq!(ws.root, outside);
    }

    #[test]
    fn config_loads_and_merges_global_and_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("global.toml");
        let local = tmp.path().join("local.toml");

        std::fs::write(
            &global,
            r#"
jobs = 4
limit_rate = "5M"
color = "auto"
api_concurrency = 6
quality = "720p"
codec = "h264"
sound_only = false
audio_format = "m4a"
recheck = false
tmp_dir = "global_tmp"
"#,
        )
        .unwrap();
        std::fs::write(
            &local,
            r#"
jobs = 2
prune = true
output_dir = "videos"
clarity = "1080p"
codec = "hevc"
sound_only = true
audio_format = "mp3"
recheck = true
tmp_dir = "staging"
"#,
        )
        .unwrap();

        let cfg = Config::load_from_paths(Some(&global), Some(&local)).unwrap();
        assert_eq!(cfg.jobs, Some(2));
        assert_eq!(cfg.limit_rate.as_deref(), Some("5M"));
        assert_eq!(cfg.color.as_deref(), Some("auto"));
        assert_eq!(cfg.prune, Some(true));
        assert_eq!(cfg.api_concurrency, Some(6));
        assert_eq!(cfg.output_dir, Some(PathBuf::from("videos")));
        assert_eq!(cfg.parsed_limit_rate().unwrap(), Some(5 * 1024 * 1024));
        assert_eq!(cfg.parsed_clarity().unwrap(), Some(VideoClarity::P1080));
        assert_eq!(cfg.parsed_codec().unwrap(), Some(VideoCodec::Hevc));
        assert_eq!(cfg.sound_only, Some(true));
        assert_eq!(cfg.parsed_audio_format().unwrap(), Some(AudioFormat::Mp3));
        assert_eq!(cfg.recheck, Some(true));
        assert_eq!(cfg.tmp_dir, Some(PathBuf::from("staging")));

        // Global `quality` alias works when not overridden by workspace `clarity`.
        let global_only = Config::load_from_paths(Some(&global), None).unwrap();
        assert_eq!(
            global_only.parsed_clarity().unwrap(),
            Some(VideoClarity::P720)
        );
        assert_eq!(global_only.parsed_codec().unwrap(), Some(VideoCodec::Avc));
        assert_eq!(
            global_only.parsed_audio_format().unwrap(),
            Some(AudioFormat::M4a)
        );

        // resolved_tmp_dir handles None, relative, and absolute paths.
        let ws = Workspace::new(tmp.path().join("ws"));
        assert_eq!(Config::default().resolved_tmp_dir(&ws), ws.tmp_dir());
        assert_eq!(cfg.resolved_tmp_dir(&ws), ws.root().join("staging"));
        let abs_cfg = Config {
            tmp_dir: Some(tmp.path().join("abs_tmp")),
            ..Default::default()
        };
        assert_eq!(abs_cfg.resolved_tmp_dir(&ws), tmp.path().join("abs_tmp"));

        // Invalid values produce Usage errors.
        let bad_clarity = Config {
            clarity: Some("invalid".to_owned()),
            ..Default::default()
        };
        assert!(bad_clarity.parsed_clarity().is_err());
        let bad_codec = Config {
            codec: Some("vp9".to_owned()),
            ..Default::default()
        };
        assert!(bad_codec.parsed_codec().is_err());
        let bad_audio = Config {
            audio_format: Some("wav".to_owned()),
            ..Default::default()
        };
        assert!(bad_audio.parsed_audio_format().is_err());
    }

    #[test]
    fn malformed_toml_config_classifies_as_usage_error() {
        use crate::error::{ExitKind, classify_anyhow};

        let tmp = tempfile::tempdir().unwrap();
        let syntax_err_path = tmp.path().join("syntax.toml");
        std::fs::write(&syntax_err_path, "jobs = [unclosed\n").unwrap();
        let err = Config::load_file(&syntax_err_path).unwrap_err();
        assert_eq!(classify_anyhow(&err), ExitKind::Usage);

        let type_err_path = tmp.path().join("bad_type.toml");
        std::fs::write(&type_err_path, "jobs = \"not_a_number\"\n").unwrap();
        let err = Config::load_file(&type_err_path).unwrap_err();
        assert_eq!(classify_anyhow(&err), ExitKind::Usage);
    }

    #[test]
    fn rate_limit_parser_handles_units_and_rejects_bad_values() {
        assert_eq!(parse_rate_limit("1024").unwrap(), 1024);
        assert_eq!(parse_rate_limit("500K").unwrap(), 500 * 1024);
        assert_eq!(parse_rate_limit("2.5MB/s").unwrap(), 2_621_440);
        assert_eq!(parse_rate_limit("1G").unwrap(), 1024 * 1024 * 1024);
        assert!(parse_rate_limit("").is_err());
        assert!(parse_rate_limit("0").is_err());
        assert!(parse_rate_limit("-5M").is_err());
        assert!(parse_rate_limit("abc").is_err());
    }

    #[test]
    fn workspace_lock_prevents_concurrent_holders_and_releases_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        let lock1 = ws.acquire_lock().unwrap();
        assert!(ws.acquire_lock().is_err());
        drop(lock1);
        let _lock2 = ws.acquire_lock().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_permissions_enforced_private() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = ws.db_path();
        let _f = open_private_file(&db).unwrap();
        enforce_db_permissions(&db).unwrap();

        let dir_mode = std::fs::metadata(ws.fav_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let tmp_mode = std::fs::metadata(ws.tmp_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let db_mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(tmp_mode, 0o700);
        assert_eq!(db_mode, 0o600);
    }
}
