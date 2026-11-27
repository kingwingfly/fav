use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr as _,
};

use anyhow::Result;
use serde::Serialize;
use tracing::warn;

use crate::{
    config::{Config, Workspace, global_config_path},
    context::{DEFAULT_API_CONCURRENCY, DEFAULT_JOBS, RuntimeOptions},
    error::{AppError, AppResult},
    state::{AudioFormat, VideoClarity},
    table::{
        ColorChoice, OutputFormat, TableOptions, render_ids, render_jsonl, render_tsv, table,
        write_stdout_line,
    },
};

/// Provenance of an effective configuration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    Workspace,
    Global,
    Default,
}

impl ConfigSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Global => "global",
            Self::Default => "default",
        }
    }
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One effective configuration key with its rendered value and provenance source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigEntry {
    pub key: &'static str,
    pub value: String,
    pub source: ConfigSource,
}

/// Structured output for `fav config show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigShowReport {
    pub global_config_path: Option<String>,
    pub global_config_exists: bool,
    pub workspace: Option<String>,
    pub workspace_config_path: Option<String>,
    pub workspace_config_exists: bool,
    pub entries: Vec<ConfigEntry>,
}

/// One config file path record for `fav config path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigPathEntry {
    pub scope: &'static str,
    pub path: Option<String>,
    pub exists: bool,
}

/// Structured output for `fav config path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigPathReport {
    pub global: ConfigPathEntry,
    pub workspace: ConfigPathEntry,
}

fn discover_optional_workspace(runtime: &RuntimeOptions) -> Option<Workspace> {
    match Workspace::discover_existing(runtime.explicit_dir.as_deref()) {
        Ok(ws) => Some(ws),
        Err(e) => {
            if runtime.explicit_dir.is_some() {
                warn!(
                    "No workspace found at `-C/--dir` ({e:#}); showing global and default settings only."
                );
            }
            None
        }
    }
}

fn resolve_path_value(path: &Path, workspace: Option<&Workspace>) -> String {
    if path.is_absolute() {
        path.display().to_string()
    } else if let Some(ws) = workspace {
        ws.root().join(path).display().to_string()
    } else {
        path.display().to_string()
    }
}

fn pick_entry<T, F>(
    key: &'static str,
    ws_val: Option<&T>,
    global_val: Option<&T>,
    default_val: impl FnOnce() -> String,
    format_val: F,
) -> ConfigEntry
where
    F: FnOnce(&T) -> String,
{
    if let Some(v) = ws_val {
        ConfigEntry {
            key,
            value: format_val(v),
            source: ConfigSource::Workspace,
        }
    } else if let Some(v) = global_val {
        ConfigEntry {
            key,
            value: format_val(v),
            source: ConfigSource::Global,
        }
    } else {
        ConfigEntry {
            key,
            value: default_val(),
            source: ConfigSource::Default,
        }
    }
}

/// Build the effective configuration report from explicit global config path and optional workspace
/// (zero workspace lock, zero DB writes).
pub fn build_config_show_report(
    global_path: Option<&Path>,
    workspace: Option<&Workspace>,
) -> Result<ConfigShowReport> {
    let global_exists = global_path.is_some_and(Path::is_file);
    let global_cfg = match global_path {
        Some(p) => Config::load_file(p)?.unwrap_or_default(),
        None => Config::default(),
    };

    let ws_config_path: Option<PathBuf> = workspace.map(Workspace::config_path);
    let ws_config_exists = ws_config_path.as_deref().is_some_and(Path::is_file);
    let ws_cfg = match ws_config_path.as_deref() {
        Some(p) => Config::load_file(p)?.unwrap_or_default(),
        None => Config::default(),
    };

    let entries = vec![
        pick_entry(
            "output_dir",
            ws_cfg.output_dir.as_ref(),
            global_cfg.output_dir.as_ref(),
            || {
                workspace
                    .map(|ws| ws.root().display().to_string())
                    .unwrap_or_else(|| "(workspace root)".to_owned())
            },
            |p| resolve_path_value(p, workspace),
        ),
        pick_entry(
            "tmp_dir",
            ws_cfg.tmp_dir.as_ref(),
            global_cfg.tmp_dir.as_ref(),
            || {
                workspace
                    .map(|ws| ws.tmp_dir().display().to_string())
                    .unwrap_or_else(|| "(.fav/tmp)".to_owned())
            },
            |p| resolve_path_value(p, workspace),
        ),
        pick_entry(
            "jobs",
            ws_cfg.jobs.as_ref(),
            global_cfg.jobs.as_ref(),
            || DEFAULT_JOBS.to_string(),
            ToString::to_string,
        ),
        pick_entry(
            "api_concurrency",
            ws_cfg.api_concurrency.as_ref(),
            global_cfg.api_concurrency.as_ref(),
            || DEFAULT_API_CONCURRENCY.to_string(),
            ToString::to_string,
        ),
        pick_entry(
            "limit_rate",
            ws_cfg.limit_rate.as_ref(),
            global_cfg.limit_rate.as_ref(),
            || "unlimited".to_owned(),
            Clone::clone,
        ),
        pick_entry(
            "clarity",
            ws_cfg.clarity.as_ref(),
            global_cfg.clarity.as_ref(),
            || VideoClarity::default().as_str().to_owned(),
            Clone::clone,
        ),
        pick_entry(
            "codec",
            ws_cfg.codec.as_ref(),
            global_cfg.codec.as_ref(),
            || "auto".to_owned(),
            Clone::clone,
        ),
        pick_entry(
            "sound_only",
            ws_cfg.sound_only.as_ref(),
            global_cfg.sound_only.as_ref(),
            || "false".to_owned(),
            ToString::to_string,
        ),
        pick_entry(
            "audio_format",
            ws_cfg.audio_format.as_ref(),
            global_cfg.audio_format.as_ref(),
            || AudioFormat::default().as_str().to_owned(),
            Clone::clone,
        ),
        pick_entry(
            "recheck",
            ws_cfg.recheck.as_ref(),
            global_cfg.recheck.as_ref(),
            || "false".to_owned(),
            ToString::to_string,
        ),
        pick_entry(
            "prune",
            ws_cfg.prune.as_ref(),
            global_cfg.prune.as_ref(),
            || "false".to_owned(),
            ToString::to_string,
        ),
        pick_entry(
            "no_progress",
            ws_cfg.no_progress.as_ref(),
            global_cfg.no_progress.as_ref(),
            || "false".to_owned(),
            ToString::to_string,
        ),
        pick_entry(
            "color",
            ws_cfg.color.as_ref(),
            global_cfg.color.as_ref(),
            || ColorChoice::default().to_string(),
            Clone::clone,
        ),
        pick_entry(
            "ffmpeg_path",
            ws_cfg.ffmpeg_path.as_ref(),
            global_cfg.ffmpeg_path.as_ref(),
            || "ffmpeg".to_owned(),
            |p| p.display().to_string(),
        ),
    ];

    Ok(ConfigShowReport {
        global_config_path: global_path.map(|p| p.display().to_string()),
        global_config_exists: global_exists,
        workspace: workspace.map(|ws| ws.root().display().to_string()),
        workspace_config_path: ws_config_path.map(|p| p.display().to_string()),
        workspace_config_exists: ws_config_exists,
        entries,
    })
}

/// Render a [`ConfigShowReport`] in the requested [`OutputFormat`].
pub fn render_config_show(
    report: &ConfigShowReport,
    format: OutputFormat,
    table_opts: TableOptions,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(report)?),
        OutputFormat::Jsonl => render_jsonl(&report.entries),
        OutputFormat::Tsv => Ok(render_tsv(
            ["key", "value", "source"],
            report
                .entries
                .iter()
                .map(|e| [e.key, e.value.as_str(), e.source.as_str()]),
        )),
        OutputFormat::Ids => Ok(render_ids(report.entries.iter().map(|e| e.key))),
        OutputFormat::Table => {
            let global_line = match report.global_config_path.as_deref() {
                Some(p) if report.global_config_exists => format!("{p} (exists)"),
                Some(p) => format!("{p} (not found)"),
                None => "(unavailable)".to_owned(),
            };
            let ws_line = report.workspace.as_deref().unwrap_or("(none)").to_owned();
            let ws_cfg_line = match report.workspace_config_path.as_deref() {
                Some(p) if report.workspace_config_exists => format!("{p} (exists)"),
                Some(p) => format!("{p} (not found)"),
                None => "(none)".to_owned(),
            };

            let mut tbl = table(
                ["key", "value", "source"],
                report.entries.iter().map(|e| {
                    [
                        e.key.to_owned(),
                        e.value.clone(),
                        e.source.as_str().to_owned(),
                    ]
                }),
            );
            if !table_opts.wide
                && let Some(max_width) = table_opts.term_width
            {
                tbl.fit_to_width(max_width);
            }

            Ok(format!(
                "Global Config   : {global_line}\n\
                 Workspace       : {ws_line}\n\
                 Workspace Config: {ws_cfg_line}\n\n\
                 {tbl}"
            ))
        }
    }
}

/// Build the config file paths report from explicit global config path and optional workspace.
pub fn build_config_path_report(
    global_path: Option<&Path>,
    workspace: Option<&Workspace>,
) -> ConfigPathReport {
    let global = ConfigPathEntry {
        scope: "global",
        path: global_path.map(|p| p.display().to_string()),
        exists: global_path.is_some_and(Path::is_file),
    };
    let ws_path = workspace.map(Workspace::config_path);
    let workspace_entry = ConfigPathEntry {
        scope: "workspace",
        exists: ws_path.as_deref().is_some_and(Path::is_file),
        path: ws_path.map(|p| p.display().to_string()),
    };
    ConfigPathReport {
        global,
        workspace: workspace_entry,
    }
}

/// Render a [`ConfigPathReport`] in the requested [`OutputFormat`].
pub fn render_config_path(
    report: &ConfigPathReport,
    format: OutputFormat,
    table_opts: TableOptions,
) -> Result<String> {
    let entries = [&report.global, &report.workspace];
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(report)?),
        OutputFormat::Jsonl => render_jsonl(&entries),
        OutputFormat::Tsv => Ok(render_tsv(
            ["scope", "path", "exists"],
            entries.iter().map(|e| {
                [
                    e.scope,
                    e.path.as_deref().unwrap_or(""),
                    if e.exists { "true" } else { "false" },
                ]
            }),
        )),
        OutputFormat::Ids => Ok(render_ids(entries.iter().filter_map(|e| e.path.as_deref()))),
        OutputFormat::Table => {
            let mut tbl = table(
                ["scope", "path", "exists"],
                entries.iter().map(|e| {
                    [
                        e.scope.to_owned(),
                        e.path.clone().unwrap_or_else(|| "(none)".to_owned()),
                        if e.exists {
                            "yes".to_owned()
                        } else {
                            "no".to_owned()
                        },
                    ]
                }),
            );
            if !table_opts.wide
                && let Some(max_width) = table_opts.term_width
            {
                tbl.fit_to_width(max_width);
            }
            Ok(tbl.to_string())
        }
    }
}

/// Entry point for `fav config` / `fav config show` (read-only, zero DB writes, zero lock).
pub fn config_show(runtime: &RuntimeOptions, format: OutputFormat) -> AppResult<()> {
    let workspace = discover_optional_workspace(runtime);
    let global_path = global_config_path();
    let report = build_config_show_report(global_path.as_deref(), workspace.as_ref())?;

    let effective_color = if runtime.color == ColorChoice::Auto {
        report
            .entries
            .iter()
            .find(|e| e.key == "color")
            .and_then(|e| ColorChoice::from_str(&e.value).ok())
            .unwrap_or(runtime.color)
    } else {
        runtime.color
    };
    let table_opts = TableOptions::detect(false, effective_color);
    let out = render_config_show(&report, format, table_opts)?;
    if !out.is_empty() {
        write_stdout_line(&out).map_err(AppError::runtime)?;
    }
    Ok(())
}

/// Entry point for `fav config path` (read-only, zero DB writes, zero lock).
pub fn config_path(runtime: &RuntimeOptions, format: OutputFormat) -> AppResult<()> {
    let workspace = discover_optional_workspace(runtime);
    let global_path = global_config_path();
    let report = build_config_path_report(global_path.as_deref(), workspace.as_ref());
    let table_opts = TableOptions::detect(false, runtime.color);
    let out = render_config_path(&report, format, table_opts)?;
    if !out.is_empty() {
        write_stdout_line(&out).map_err(AppError::runtime)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_table_opts() -> TableOptions {
        TableOptions {
            wide: true,
            is_tty: false,
            colored: false,
            term_width: None,
        }
    }

    #[test]
    fn config_show_provenance_precedence_workspace_over_global_over_default() {
        let tmp = tempfile::tempdir().unwrap();
        let global_path = tmp.path().join("global.toml");
        let ws_root = tmp.path().join("ws");
        let ws = Workspace::new(ws_root.clone());
        std::fs::create_dir_all(ws.fav_dir()).unwrap();

        std::fs::write(
            &global_path,
            r#"
output_dir = "global_videos"
tmp_dir = "global_tmp"
jobs = 6
api_concurrency = 8
limit_rate = "5M"
quality = "720p"
codec = "avc"
sound_only = false
audio_format = "m4a"
recheck = false
prune = true
no_progress = false
color = "always"
ffmpeg_path = "/opt/global/ffmpeg"
"#,
        )
        .unwrap();

        // Workspace overrides a subset of keys; remaining keys come from global.
        std::fs::write(
            ws.config_path(),
            r#"
output_dir = "ws_videos"
jobs = 2
clarity = "1080p"
sound_only = true
audio_format = "opus"
recheck = true
no_progress = true
"#,
        )
        .unwrap();

        let report = build_config_show_report(Some(&global_path), Some(&ws)).unwrap();
        assert!(report.global_config_exists);
        assert!(report.workspace_config_exists);
        assert_eq!(report.entries.len(), 14);

        let by_key = |k: &str| -> &ConfigEntry {
            report
                .entries
                .iter()
                .find(|e| e.key == k)
                .unwrap_or_else(|| panic!("missing key {k}"))
        };

        // Workspace wins over global
        assert_eq!(
            by_key("output_dir").value,
            ws_root.join("ws_videos").display().to_string()
        );
        assert_eq!(by_key("output_dir").source, ConfigSource::Workspace);
        assert_eq!(by_key("jobs").value, "2");
        assert_eq!(by_key("jobs").source, ConfigSource::Workspace);
        assert_eq!(by_key("clarity").value, "1080p");
        assert_eq!(by_key("clarity").source, ConfigSource::Workspace);
        assert_eq!(by_key("sound_only").value, "true");
        assert_eq!(by_key("sound_only").source, ConfigSource::Workspace);
        assert_eq!(by_key("audio_format").value, "opus");
        assert_eq!(by_key("audio_format").source, ConfigSource::Workspace);
        assert_eq!(by_key("recheck").value, "true");
        assert_eq!(by_key("recheck").source, ConfigSource::Workspace);
        assert_eq!(by_key("no_progress").value, "true");
        assert_eq!(by_key("no_progress").source, ConfigSource::Workspace);

        // Global wins over default
        assert_eq!(
            by_key("tmp_dir").value,
            ws_root.join("global_tmp").display().to_string()
        );
        assert_eq!(by_key("tmp_dir").source, ConfigSource::Global);
        assert_eq!(by_key("api_concurrency").value, "8");
        assert_eq!(by_key("api_concurrency").source, ConfigSource::Global);
        assert_eq!(by_key("limit_rate").value, "5M");
        assert_eq!(by_key("limit_rate").source, ConfigSource::Global);
        assert_eq!(by_key("codec").value, "avc");
        assert_eq!(by_key("codec").source, ConfigSource::Global);
        assert_eq!(by_key("prune").value, "true");
        assert_eq!(by_key("prune").source, ConfigSource::Global);
        assert_eq!(by_key("color").value, "always");
        assert_eq!(by_key("color").source, ConfigSource::Global);
        assert_eq!(by_key("ffmpeg_path").value, "/opt/global/ffmpeg");
        assert_eq!(by_key("ffmpeg_path").source, ConfigSource::Global);

        // Now test defaults when neither config file exists.
        let missing_global = tmp.path().join("missing-global.toml");
        let empty_ws = Workspace::new(tmp.path().join("empty-ws"));
        let defaults = build_config_show_report(Some(&missing_global), Some(&empty_ws)).unwrap();
        assert!(!defaults.global_config_exists);
        assert!(!defaults.workspace_config_exists);
        assert_eq!(defaults.entries.len(), 14);
        for entry in &defaults.entries {
            assert_eq!(
                entry.source,
                ConfigSource::Default,
                "expected default source for {}",
                entry.key
            );
        }
        let def_by_key = |k: &str| defaults.entries.iter().find(|e| e.key == k).unwrap();
        assert_eq!(
            def_by_key("output_dir").value,
            empty_ws.root().display().to_string()
        );
        assert_eq!(
            def_by_key("tmp_dir").value,
            empty_ws.tmp_dir().display().to_string()
        );
        assert_eq!(def_by_key("jobs").value, DEFAULT_JOBS.to_string());
        assert_eq!(
            def_by_key("api_concurrency").value,
            DEFAULT_API_CONCURRENCY.to_string()
        );
        assert_eq!(def_by_key("limit_rate").value, "unlimited");
        assert_eq!(def_by_key("clarity").value, "best");
        assert_eq!(def_by_key("codec").value, "auto");
        assert_eq!(def_by_key("sound_only").value, "false");
        assert_eq!(def_by_key("audio_format").value, "copy");
        assert_eq!(def_by_key("recheck").value, "false");
        assert_eq!(def_by_key("prune").value, "false");
        assert_eq!(def_by_key("no_progress").value, "false");
        assert_eq!(def_by_key("color").value, "auto");
        assert_eq!(def_by_key("ffmpeg_path").value, "ffmpeg");
    }

    #[test]
    fn config_show_and_path_render_table_json_jsonl_tsv_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let global_path = tmp.path().join("config.toml");
        let ws = Workspace::new(tmp.path().join("ws"));
        std::fs::create_dir_all(ws.fav_dir()).unwrap();
        std::fs::write(&global_path, "jobs = 5\ncolor = \"never\"\n").unwrap();
        std::fs::write(ws.config_path(), "prune = true\n").unwrap();

        let report = build_config_show_report(Some(&global_path), Some(&ws)).unwrap();

        let table_out =
            render_config_show(&report, OutputFormat::Table, plain_table_opts()).unwrap();
        assert!(table_out.contains("Global Config"));
        assert!(table_out.contains("(exists)"));
        assert!(table_out.contains("| key"));
        assert!(table_out.contains("| jobs"));
        assert!(table_out.contains("| global"));
        assert!(table_out.contains("| workspace"));
        assert!(table_out.contains("| default"));

        let json_out = render_config_show(&report, OutputFormat::Json, plain_table_opts()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_out).unwrap();
        assert_eq!(parsed["global_config_exists"], true);
        assert_eq!(parsed["workspace_config_exists"], true);
        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 14);
        let jobs_obj = entries.iter().find(|e| e["key"] == "jobs").unwrap();
        assert_eq!(jobs_obj["value"], "5");
        assert_eq!(jobs_obj["source"], "global");
        let prune_obj = entries.iter().find(|e| e["key"] == "prune").unwrap();
        assert_eq!(prune_obj["value"], "true");
        assert_eq!(prune_obj["source"], "workspace");

        let jsonl_out =
            render_config_show(&report, OutputFormat::Jsonl, plain_table_opts()).unwrap();
        assert_eq!(jsonl_out.lines().count(), 14);

        let tsv_out = render_config_show(&report, OutputFormat::Tsv, plain_table_opts()).unwrap();
        assert!(tsv_out.starts_with("key\tvalue\tsource\n"));
        assert!(tsv_out.contains("jobs\t5\tglobal"));

        let ids_out = render_config_show(&report, OutputFormat::Ids, plain_table_opts()).unwrap();
        assert_eq!(ids_out.lines().count(), 14);
        assert!(ids_out.lines().any(|l| l == "output_dir"));
        assert!(ids_out.lines().any(|l| l == "ffmpeg_path"));

        // `fav config path` report & rendering
        let path_report = build_config_path_report(Some(&global_path), Some(&ws));
        assert!(path_report.global.exists);
        assert!(path_report.workspace.exists);

        let path_table =
            render_config_path(&path_report, OutputFormat::Table, plain_table_opts()).unwrap();
        assert!(path_table.contains("global"));
        assert!(path_table.contains("workspace"));
        assert!(path_table.contains("yes"));

        let path_json =
            render_config_path(&path_report, OutputFormat::Json, plain_table_opts()).unwrap();
        let path_val: serde_json::Value = serde_json::from_str(&path_json).unwrap();
        assert_eq!(path_val["global"]["exists"], true);
        assert_eq!(path_val["workspace"]["exists"], true);

        let path_ids =
            render_config_path(&path_report, OutputFormat::Ids, plain_table_opts()).unwrap();
        assert_eq!(path_ids.lines().count(), 2);
    }

    #[test]
    fn config_works_cleanly_outside_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let global_path = tmp.path().join("global.toml");
        std::fs::write(&global_path, "jobs = 7\nclarity = \"4k\"\n").unwrap();

        let report = build_config_show_report(Some(&global_path), None).unwrap();
        assert!(report.global_config_exists);
        assert!(report.workspace.is_none());
        assert!(report.workspace_config_path.is_none());
        assert!(!report.workspace_config_exists);
        assert_eq!(report.entries.len(), 14);

        let by_key = |k: &str| report.entries.iter().find(|e| e.key == k).unwrap();
        assert_eq!(by_key("output_dir").value, "(workspace root)");
        assert_eq!(by_key("output_dir").source, ConfigSource::Default);
        assert_eq!(by_key("tmp_dir").value, "(.fav/tmp)");
        assert_eq!(by_key("tmp_dir").source, ConfigSource::Default);
        assert_eq!(by_key("jobs").value, "7");
        assert_eq!(by_key("jobs").source, ConfigSource::Global);
        assert_eq!(by_key("clarity").value, "4k");
        assert_eq!(by_key("clarity").source, ConfigSource::Global);

        let table_out =
            render_config_show(&report, OutputFormat::Table, plain_table_opts()).unwrap();
        assert!(table_out.contains("Workspace       : (none)"));
        assert!(table_out.contains("Workspace Config: (none)"));

        let path_report = build_config_path_report(Some(&global_path), None);
        assert!(path_report.global.exists);
        assert!(path_report.workspace.path.is_none());
        assert!(!path_report.workspace.exists);

        let path_json =
            render_config_path(&path_report, OutputFormat::Json, plain_table_opts()).unwrap();
        let path_val: serde_json::Value = serde_json::from_str(&path_json).unwrap();
        assert_eq!(path_val["global"]["exists"], true);
        assert!(path_val["workspace"]["path"].is_null());
        assert_eq!(path_val["workspace"]["exists"], false);

        let path_table =
            render_config_path(&path_report, OutputFormat::Table, plain_table_opts()).unwrap();
        assert!(path_table.contains("(none)"));
        assert!(path_table.contains("no"));

        let path_ids =
            render_config_path(&path_report, OutputFormat::Ids, plain_table_opts()).unwrap();
        assert_eq!(path_ids, global_path.display().to_string());
    }
}
