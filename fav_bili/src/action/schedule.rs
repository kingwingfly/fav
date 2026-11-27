use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{ValueEnum, builder::PossibleValue};
use tracing::warn;

use crate::error::AppError;

/// Supported OS scheduler backends for `fav schedule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleTarget {
    Systemd,
    Launchd,
    TaskScheduler,
}

impl ValueEnum for ScheduleTarget {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Systemd, Self::Launchd, Self::TaskScheduler]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            Self::Systemd => PossibleValue::new("systemd"),
            Self::Launchd => PossibleValue::new("launchd"),
            Self::TaskScheduler => PossibleValue::new("task-scheduler").alias("windows"),
        })
    }
}

impl fmt::Display for ScheduleTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Systemd => "systemd",
            Self::Launchd => "launchd",
            Self::TaskScheduler => "task-scheduler",
        })
    }
}

impl FromStr for ScheduleTarget {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "systemd" => Ok(Self::Systemd),
            "launchd" => Ok(Self::Launchd),
            "task-scheduler" | "taskscheduler" | "windows" => Ok(Self::TaskScheduler),
            other => Err(format!(
                "invalid schedule target '{other}' (expected one of: systemd, launchd, task-scheduler)"
            )),
        }
    }
}

/// Validated recurring schedule interval (e.g. `30m`, `3h`, `1d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleInterval {
    pub value: u64,
    pub unit: char,
    pub seconds: u64,
}

impl ScheduleInterval {
    /// Format as a `systemd.time(7)` duration string (e.g. `30min`, `3h`, `1d`).
    pub fn systemd_spec(&self) -> String {
        match self.unit {
            'm' => format!("{}min", self.value),
            'h' => format!("{}h", self.value),
            'd' => format!("{}d", self.value),
            _ => format!("{}s", self.seconds),
        }
    }

    /// Format as a PowerShell `New-TimeSpan` expression.
    pub fn powershell_timespan(&self) -> String {
        match self.unit {
            'm' => format!("New-TimeSpan -Minutes {}", self.value),
            'h' => format!("New-TimeSpan -Hours {}", self.value),
            'd' => format!("New-TimeSpan -Days {}", self.value),
            _ => format!("New-TimeSpan -Seconds {}", self.seconds),
        }
    }
}

impl fmt::Display for ScheduleInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit)
    }
}

/// Parse and validate an interval string such as `30m`, `3h`, or `1d`.
pub fn parse_interval(input: &str) -> Result<ScheduleInterval> {
    let trimmed = input.trim();
    if trimmed.len() < 2 || !trimmed.is_ascii() {
        bail!("invalid interval '{input}' (expected format like 30m, 3h, or 1d)");
    }
    let unit = trimmed
        .as_bytes()
        .last()
        .copied()
        .ok_or_else(|| anyhow!("invalid interval '{input}'"))?
        .to_ascii_lowercase() as char;
    let num_str = &trimmed[..trimmed.len() - 1];
    if num_str.is_empty() || !num_str.bytes().all(|b| b.is_ascii_digit()) {
        bail!("invalid interval '{input}' (expected positive integer followed by m, h, or d)");
    }
    let value: u64 = num_str
        .parse()
        .map_err(|_| anyhow!("interval value out of range: '{input}'"))?;
    if value == 0 {
        bail!("interval must be greater than zero: '{input}'");
    }

    let multiplier: u64 = match unit {
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => {
            bail!("unsupported interval unit '{unit}' in '{input}' (supported units: m, h, d)");
        }
    };
    let seconds = value
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow!("interval overflow: '{input}'"))?;

    const MAX_SECONDS: u64 = 365 * 86_400;
    if seconds > MAX_SECONDS {
        bail!("interval '{input}' exceeds maximum supported duration of 365d");
    }

    Ok(ScheduleInterval {
        value,
        unit,
        seconds,
    })
}

/// One generated scheduler configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleFile {
    pub filename: String,
    pub content: String,
}

/// Output of [`generate_schedule`]: generated files plus user-facing activation instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSchedule {
    pub files: Vec<ScheduleFile>,
    pub instructions: String,
}

fn escape_systemd_arg(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut escaped = String::with_capacity(raw.len() + 2);
    escaped.push('"');
    for ch in raw.chars() {
        match ch {
            '\\' | '"' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            '%' => escaped.push_str("%%"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn escape_powershell_single(s: &str) -> String {
    s.replace('\'', "''")
}

/// Validate an optional `--name` suffix for `fav schedule`.
///
/// Must match `^[A-Za-z0-9][A-Za-z0-9_-]{0,47}$` (1..=48 ASCII chars).
pub fn validate_schedule_name(name: &str) -> Result<&str> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 48
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
    {
        return Err(AppError::usage(anyhow!(
            "invalid schedule --name '{name}' (must match ^[A-Za-z0-9][A-Za-z0-9_-]{{0,47}}$)"
        ))
        .into());
    }
    Ok(name)
}

/// Generate OS scheduler templates without touching the filesystem using default unit/task names.
pub fn generate_schedule(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
) -> Result<GeneratedSchedule> {
    generate_schedule_named(target, interval, workspace_dir, fav_bin, None)
}

/// Generate OS scheduler templates without touching the filesystem, optionally suffixing
/// unit/label/task names with `name` so multiple workspaces can coexist cleanly.
pub fn generate_schedule_named(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
    name: Option<&str>,
) -> Result<GeneratedSchedule> {
    let parsed = parse_interval(interval).map_err(|e| anyhow::Error::from(AppError::usage(e)))?;
    let suffix = match name {
        Some(raw) => Some(validate_schedule_name(raw)?),
        None => None,
    };

    match target {
        ScheduleTarget::Systemd => {
            let unit_base = match suffix {
                Some(s) => format!("fav-{s}"),
                None => "fav".to_owned(),
            };
            let service_desc = match suffix {
                Some(s) => format!("Fav Bilibili Backup Sync ({s})"),
                None => "Fav Bilibili Backup Sync".to_owned(),
            };
            let timer_desc = match suffix {
                Some(s) => format!("Run fav sync ({s}) every {parsed}"),
                None => format!("Run fav sync every {parsed}"),
            };
            let service = format!(
                "[Unit]\n\
                 Description={service_desc}\n\
                 After=network-online.target\n\
                 Wants=network-online.target\n\
                 \n\
                 [Service]\n\
                 Type=oneshot\n\
                 ExecStart={} -C {} sync\n",
                escape_systemd_arg(fav_bin),
                escape_systemd_arg(workspace_dir),
            );
            let timer = format!(
                "[Unit]\n\
                 Description={timer_desc}\n\
                 \n\
                 [Timer]\n\
                 OnBootSec=5min\n\
                 OnUnitActiveSec={}\n\
                 RandomizedDelaySec=60\n\
                 Persistent=true\n\
                 \n\
                 [Install]\n\
                 WantedBy=timers.target\n",
                parsed.systemd_spec(),
            );
            let instructions = format!(
                "Enable and start the user timer (user-level, no root privileges required):\n\
                   systemctl --user daemon-reload\n\
                   systemctl --user enable --now {unit_base}.timer\n\
                   systemctl --user status {unit_base}.timer"
            );
            Ok(GeneratedSchedule {
                files: vec![
                    ScheduleFile {
                        filename: format!("{unit_base}.service"),
                        content: service,
                    },
                    ScheduleFile {
                        filename: format!("{unit_base}.timer"),
                        content: timer,
                    },
                ],
                instructions,
            })
        }
        ScheduleTarget::Launchd => {
            let label = match suffix {
                Some(s) => format!("com.kingwingfly.fav.{s}"),
                None => "com.kingwingfly.fav".to_owned(),
            };
            let plist_name = format!("{label}.plist");
            let bin_xml = escape_xml(&fav_bin.to_string_lossy());
            let dir_xml = escape_xml(&workspace_dir.to_string_lossy());
            let log_xml = escape_xml(&workspace_dir.join(".fav/schedule.log").to_string_lossy());
            let plist = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                 \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\">\n\
                 <dict>\n\
                   <key>Label</key>\n\
                   <string>{label}</string>\n\
                   <key>ProgramArguments</key>\n\
                   <array>\n\
                     <string>{bin_xml}</string>\n\
                     <string>-C</string>\n\
                     <string>{dir_xml}</string>\n\
                     <string>sync</string>\n\
                   </array>\n\
                   <key>StartInterval</key>\n\
                   <integer>{}</integer>\n\
                   <key>RunAtLoad</key>\n\
                   <true/>\n\
                   <key>StandardOutPath</key>\n\
                   <string>{log_xml}</string>\n\
                   <key>StandardErrorPath</key>\n\
                   <string>{log_xml}</string>\n\
                 </dict>\n\
                 </plist>\n",
                parsed.seconds,
            );
            let instructions = format!(
                "Load the LaunchAgent (user-level, no root privileges required):\n\
                   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/{plist_name}\n\
                   launchctl kickstart -k gui/$(id -u)/{label}"
            );
            Ok(GeneratedSchedule {
                files: vec![ScheduleFile {
                    filename: plist_name,
                    content: plist,
                }],
                instructions,
            })
        }
        ScheduleTarget::TaskScheduler => {
            let task_name = match suffix {
                Some(s) => format!("FavSync-{s}"),
                None => "FavSync".to_owned(),
            };
            let script_name = match suffix {
                Some(s) => format!("install-fav-task-{s}.ps1"),
                None => "install-fav-task.ps1".to_owned(),
            };
            let bin_ps = escape_powershell_single(&fav_bin.to_string_lossy());
            let dir_ps = escape_powershell_single(&workspace_dir.to_string_lossy());
            let script = format!(
                "$FavBin = '{bin_ps}'\n\
                 $WorkspaceDir = '{dir_ps}'\n\
                 $Action = New-ScheduledTaskAction -Execute $FavBin -Argument \"-C `\"$WorkspaceDir`\" sync\"\n\
                 $Trigger = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval ({})\n\
                 $Settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable\n\
                 Register-ScheduledTask -TaskName '{task_name}' -Action $Action -Trigger $Trigger -Settings $Settings -Force\n",
                parsed.powershell_timespan(),
            );
            let instructions = format!(
                "Register the scheduled task in PowerShell:\n\
                   powershell -ExecutionPolicy Bypass -File {script_name}"
            );
            Ok(GeneratedSchedule {
                files: vec![ScheduleFile {
                    filename: script_name,
                    content: script,
                }],
                instructions,
            })
        }
    }
}

fn content_mentions_workspace(content: &str, target: ScheduleTarget, workspace_dir: &Path) -> bool {
    let raw = workspace_dir.to_string_lossy();
    if raw.is_empty() {
        return false;
    }
    let exact_marker = match target {
        ScheduleTarget::Systemd => escape_systemd_arg(workspace_dir),
        ScheduleTarget::Launchd => format!("<string>{}</string>", escape_xml(&raw)),
        ScheduleTarget::TaskScheduler => format!("'{}'", escape_powershell_single(&raw)),
    };
    if content.contains(&exact_marker) {
        return true;
    }
    // Fallback for manually edited files containing the raw workspace path as a delimited token.
    let bytes = content.as_bytes();
    let needle = raw.as_bytes();
    let mut offset = 0;
    while let Some(pos) = content[offset..].find(raw.as_ref()) {
        let abs = offset + pos;
        let end = abs + needle.len();
        let prev_ok = abs == 0
            || matches!(
                bytes[abs - 1],
                b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b'>' | b'='
            );
        let next_ok = bytes
            .get(end)
            .is_none_or(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b'<'));
        if prev_ok && next_ok {
            return true;
        }
        offset = abs + 1;
    }
    false
}

fn detect_cross_workspace_overwrites(
    target: ScheduleTarget,
    generated: &GeneratedSchedule,
    workspace_dir: &Path,
    dest_dir: &Path,
) -> Vec<PathBuf> {
    let mut overwritten = Vec::new();
    for file in &generated.files {
        if !content_mentions_workspace(&file.content, target, workspace_dir) {
            continue;
        }
        let dest = dest_dir.join(&file.filename);
        if dest.is_file()
            && let Ok(existing) = fs::read_to_string(&dest)
            && !content_mentions_workspace(&existing, target, workspace_dir)
        {
            overwritten.push(dest);
        }
    }
    overwritten
}

/// Write generated schedule files into `target_dir` and return their written paths.
pub fn write_schedule_files(
    generated: &GeneratedSchedule,
    target_dir: &Path,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("failed to create directory {}", target_dir.display()))?;
    let mut written = Vec::with_capacity(generated.files.len());
    for file in &generated.files {
        let dest = target_dir.join(&file.filename);
        fs::write(&dest, &file.content)
            .with_context(|| format!("failed to write {}", dest.display()))?;
        written.push(dest);
    }
    Ok(written)
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn default_install_dir(target: ScheduleTarget, workspace_dir: &Path) -> Result<PathBuf> {
    match target {
        ScheduleTarget::Systemd => {
            let cfg = crate::config::user_config_root()
                .ok_or_else(|| anyhow!("could not determine user config directory"))?;
            Ok(cfg.join("systemd").join("user"))
        }
        ScheduleTarget::Launchd => {
            let home = user_home_dir()
                .ok_or_else(|| anyhow!("could not determine user home directory"))?;
            Ok(home.join("Library").join("LaunchAgents"))
        }
        ScheduleTarget::TaskScheduler => Ok(workspace_dir.join(".fav")),
    }
}

/// Generate (and optionally install) a user-level schedule configuration using default
/// unit/task names, returning human-readable output ready to print.
pub fn schedule(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
    install: bool,
) -> Result<String> {
    schedule_named(target, interval, workspace_dir, fav_bin, install, None)
}

/// Generate (and optionally install) a user-level schedule configuration with an optional
/// `--name` suffix for managing multiple workspaces cleanly.
pub fn schedule_named(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
    install: bool,
    name: Option<&str>,
) -> Result<String> {
    let dest_dir = if install {
        Some(default_install_dir(target, workspace_dir)?)
    } else {
        None
    };
    schedule_named_with_install_dir(
        target,
        interval,
        workspace_dir,
        fav_bin,
        dest_dir.as_deref(),
        name,
    )
}

fn schedule_named_with_install_dir(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
    install_dir: Option<&Path>,
    name: Option<&str>,
) -> Result<String> {
    let generated = generate_schedule_named(target, interval, workspace_dir, fav_bin, name)?;
    if let Some(dest_dir) = install_dir {
        let overwritten_other =
            detect_cross_workspace_overwrites(target, &generated, workspace_dir, dest_dir);
        let written = write_schedule_files(&generated, dest_dir)?;
        let mut out = String::new();
        out.push_str("Installed schedule files:\n");
        for path in written {
            out.push_str(&format!("  {}\n", path.display()));
        }
        if !overwritten_other.is_empty() {
            let files_list = overwritten_other
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            warn!(
                "Overwrote existing schedule file(s) ({files_list}) that referenced a different workspace; pass `--name <NAME>` to manage multiple workspaces separately."
            );
            out.push_str(&format!(
                "\nNote: Overwrote existing schedule file(s) ({files_list}) belonging to another workspace.\n\
                 To keep schedules for multiple workspaces separate, pass `--name <NAME>` (e.g. `fav schedule --name work --install`).\n"
            ));
        }
        out.push('\n');
        out.push_str(&generated.instructions);
        Ok(out)
    } else {
        let mut out = String::new();
        for (idx, file) in generated.files.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(&format!("# === {} ===\n", file.filename));
            out.push_str(&file.content);
        }
        out.push('\n');
        out.push_str(&generated.instructions);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interval_valid_and_invalid() {
        let m = parse_interval("30m").unwrap();
        assert_eq!(m.seconds, 1_800);
        assert_eq!(m.systemd_spec(), "30min");
        assert_eq!(m.powershell_timespan(), "New-TimeSpan -Minutes 30");

        let h = parse_interval("3h").unwrap();
        assert_eq!(h.seconds, 10_800);
        assert_eq!(h.systemd_spec(), "3h");
        assert_eq!(h.powershell_timespan(), "New-TimeSpan -Hours 3");

        let d = parse_interval(" 1D ").unwrap();
        assert_eq!(d.seconds, 86_400);
        assert_eq!(d.systemd_spec(), "1d");
        assert_eq!(d.powershell_timespan(), "New-TimeSpan -Days 1");

        for bad in [
            "",
            "3",
            "h",
            "0m",
            "-3h",
            "10s",
            "2w",
            "3.5h",
            "abc",
            "3小时",
            "400d",
            "999999999999999999999h",
        ] {
            assert!(
                parse_interval(bad).is_err(),
                "expected '{bad}' to fail interval parsing"
            );
        }
    }

    #[test]
    fn generate_systemd_units() {
        let sched = generate_schedule(
            ScheduleTarget::Systemd,
            "3h",
            Path::new("/home/user/Videos/fav 100%"),
            Path::new("/usr/local/bin/fav"),
        )
        .unwrap();
        assert_eq!(sched.files.len(), 2);
        assert_eq!(sched.files[0].filename, "fav.service");
        assert!(sched.files[0].content.contains("Type=oneshot"));
        assert!(
            sched.files[0].content.contains(
                "ExecStart=\"/usr/local/bin/fav\" -C \"/home/user/Videos/fav 100%%\" sync"
            )
        );
        assert_eq!(sched.files[1].filename, "fav.timer");
        assert!(sched.files[1].content.contains("OnUnitActiveSec=3h"));
        assert!(sched.files[1].content.contains("Persistent=true"));
        assert!(sched.instructions.contains("systemctl --user"));
        assert!(!sched.instructions.contains("sudo"));
    }

    #[test]
    fn generate_launchd_plist_with_xml_escaping() {
        let sched = generate_schedule(
            ScheduleTarget::Launchd,
            "30m",
            Path::new("/Users/me/A & B <fav>"),
            Path::new("/opt/homebrew/bin/fav"),
        )
        .unwrap();
        assert_eq!(sched.files.len(), 1);
        assert_eq!(sched.files[0].filename, "com.kingwingfly.fav.plist");
        let xml = &sched.files[0].content;
        assert!(xml.contains("<integer>1800</integer>"));
        assert!(xml.contains("<string>/opt/homebrew/bin/fav</string>"));
        assert!(xml.contains("<string>/Users/me/A &amp; B &lt;fav&gt;</string>"));
        assert!(xml.contains("<string>sync</string>"));
        assert!(sched.instructions.contains("launchctl"));
        assert!(!sched.instructions.contains("sudo"));
    }

    #[test]
    fn generate_task_scheduler_script() {
        let sched = generate_schedule(
            ScheduleTarget::TaskScheduler,
            "1d",
            Path::new(r"C:\Users\O'Brien\Videos"),
            Path::new(r"C:\bin\fav.exe"),
        )
        .unwrap();
        assert_eq!(sched.files.len(), 1);
        assert_eq!(sched.files[0].filename, "install-fav-task.ps1");
        let ps = &sched.files[0].content;
        assert!(ps.contains("O''Brien"));
        assert!(ps.contains("New-TimeSpan -Days 1"));
        assert!(ps.contains("Register-ScheduledTask"));
    }

    #[test]
    fn write_schedule_files_persists_to_target_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let sched = generate_schedule(
            ScheduleTarget::Systemd,
            "3h",
            Path::new("/tmp/ws"),
            Path::new("/usr/bin/fav"),
        )
        .unwrap();
        let written = write_schedule_files(&sched, tmp.path()).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(
            fs::read_to_string(tmp.path().join("fav.service")).unwrap(),
            sched.files[0].content
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join("fav.timer")).unwrap(),
            sched.files[1].content
        );
    }

    #[test]
    fn generate_schedule_named_customizes_all_three_backends() {
        let ws = Path::new("/tmp/work-archive");
        let bin = Path::new("/usr/local/bin/fav");

        let sys = generate_schedule_named(ScheduleTarget::Systemd, "2h", ws, bin, Some("work_1"))
            .unwrap();
        assert_eq!(sys.files.len(), 2);
        assert_eq!(sys.files[0].filename, "fav-work_1.service");
        assert_eq!(sys.files[1].filename, "fav-work_1.timer");
        assert!(
            sys.instructions
                .contains("systemctl --user enable --now fav-work_1.timer")
        );
        assert!(
            sys.instructions
                .contains("systemctl --user status fav-work_1.timer")
        );

        let mac = generate_schedule_named(ScheduleTarget::Launchd, "30m", ws, bin, Some("media-2"))
            .unwrap();
        assert_eq!(mac.files.len(), 1);
        assert_eq!(mac.files[0].filename, "com.kingwingfly.fav.media-2.plist");
        assert!(
            mac.files[0]
                .content
                .contains("<string>com.kingwingfly.fav.media-2</string>")
        );
        assert!(
            mac.instructions
                .contains("~/Library/LaunchAgents/com.kingwingfly.fav.media-2.plist")
        );
        assert!(
            mac.instructions
                .contains("gui/$(id -u)/com.kingwingfly.fav.media-2")
        );

        let win = generate_schedule_named(
            ScheduleTarget::TaskScheduler,
            "1d",
            ws,
            bin,
            Some("Personal"),
        )
        .unwrap();
        assert_eq!(win.files.len(), 1);
        assert_eq!(win.files[0].filename, "install-fav-task-Personal.ps1");
        assert!(
            win.files[0]
                .content
                .contains("-TaskName 'FavSync-Personal'")
        );
        assert!(win.instructions.contains("install-fav-task-Personal.ps1"));
    }

    #[test]
    fn validate_schedule_name_accepts_valid_and_rejects_invalid_with_usage_error() {
        use crate::error::{ExitKind, classify_anyhow};

        let max_len = "A".repeat(48);
        for ok in ["a", "0", "work", "my-ws_01", max_len.as_str()] {
            assert_eq!(validate_schedule_name(ok).unwrap(), ok);
        }

        let too_long = "a".repeat(49);
        for bad in [
            "",
            "-leading-dash",
            "_leading_under",
            "has space",
            "has.dot",
            "../escape",
            "sub/dir",
            "中文",
            too_long.as_str(),
        ] {
            let err = generate_schedule_named(
                ScheduleTarget::Systemd,
                "3h",
                Path::new("/tmp/ws"),
                Path::new("/usr/bin/fav"),
                Some(bad),
            )
            .unwrap_err();
            assert_eq!(
                classify_anyhow(&err),
                ExitKind::Usage,
                "expected Usage error for --name '{bad}', got {err:#}"
            );
        }
    }

    #[test]
    fn install_detects_cross_workspace_overwrite_without_false_positives() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("units");
        let ws_a = tmp.path().join("ws");
        let ws_b = tmp.path().join("ws2");
        let bin = Path::new("/usr/local/bin/fav");

        // 1. Fresh install for ws_a: no overwrite note.
        let out1 = schedule_named_with_install_dir(
            ScheduleTarget::Systemd,
            "3h",
            &ws_a,
            bin,
            Some(&install_dir),
            None,
        )
        .unwrap();
        assert!(
            !out1.contains("Note:"),
            "fresh install must not warn: {out1}"
        );

        // 2. Re-install for SAME ws_a (including fav.timer which has no workspace path): no warning.
        let out2 = schedule_named_with_install_dir(
            ScheduleTarget::Systemd,
            "6h",
            &ws_a,
            bin,
            Some(&install_dir),
            None,
        )
        .unwrap();
        assert!(
            !out2.contains("Note:"),
            "same-workspace reinstall must not warn: {out2}"
        );

        // 3. Install for DIFFERENT ws_b with same default name -> warns and includes Note suggesting --name!
        let out3 = schedule_named_with_install_dir(
            ScheduleTarget::Systemd,
            "3h",
            &ws_b,
            bin,
            Some(&install_dir),
            None,
        )
        .unwrap();
        assert!(out3.contains("Note:"), "expected overwrite Note: {out3}");
        assert!(
            out3.contains("--name <NAME>"),
            "expected --name hint in Note: {out3}"
        );

        // 4. Prefix-path edge case: ws_a (".../ws") overwriting ws_b (".../ws2") must ALSO warn!
        let out4 = schedule_named_with_install_dir(
            ScheduleTarget::Systemd,
            "3h",
            &ws_a,
            bin,
            Some(&install_dir),
            None,
        )
        .unwrap();
        assert!(
            out4.contains("Note:"),
            "prefix path ws overwriting ws2 must warn: {out4}"
        );

        // 5. Installing ws_b with a separate `--name work` does NOT overwrite ws_a and emits no warning.
        let out5 = schedule_named_with_install_dir(
            ScheduleTarget::Systemd,
            "3h",
            &ws_b,
            bin,
            Some(&install_dir),
            Some("work"),
        )
        .unwrap();
        assert!(
            !out5.contains("Note:"),
            "distinct --name install must not warn: {out5}"
        );
        assert!(install_dir.join("fav.service").is_file());
        assert!(install_dir.join("fav-work.service").is_file());
        assert!(install_dir.join("fav-work.timer").is_file());

        // 6. Launchd cross-workspace overwrite detection.
        let mac_dir = tmp.path().join("launchd");
        let mac1 = schedule_named_with_install_dir(
            ScheduleTarget::Launchd,
            "1h",
            &ws_a,
            bin,
            Some(&mac_dir),
            None,
        )
        .unwrap();
        assert!(!mac1.contains("Note:"));
        let mac2 = schedule_named_with_install_dir(
            ScheduleTarget::Launchd,
            "1h",
            &ws_b,
            bin,
            Some(&mac_dir),
            None,
        )
        .unwrap();
        assert!(mac2.contains("Note:"));
        assert!(mac2.contains("--name <NAME>"));
    }
}
