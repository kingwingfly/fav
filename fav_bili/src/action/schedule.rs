use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{ValueEnum, builder::PossibleValue};

use crate::error::{AppError, AppResult};

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

/// Quote a single argument according to Windows `CommandLineToArgvW` rules.
///
/// Wraps `arg` in double quotes, doubling every run of `\` immediately preceding `"`
/// (escaping `"` as `\"`) and doubling any trailing run of `\` before the closing `"`.
fn escape_windows_argv(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            for _ in 0..backslashes * 2 {
                out.push('\\');
            }
            out.push_str("\\\"");
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                out.push('\\');
            }
            backslashes = 0;
            out.push(ch);
        }
    }
    for _ in 0..backslashes * 2 {
        out.push('\\');
    }
    out.push('"');
    out
}

/// Generate OS scheduler templates without touching the filesystem.
pub fn generate_schedule(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
) -> AppResult<GeneratedSchedule> {
    let parsed = parse_interval(interval).map_err(AppError::usage)?;

    match target {
        ScheduleTarget::Systemd => {
            let service = format!(
                "[Unit]\n\
                 Description=Fav Bilibili Backup Sync\n\
                 After=network-online.target\n\
                 Wants=network-online.target\n\
                 \n\
                 [Service]\n\
                 Type=oneshot\n\
                 ExecStart={} -C {} --non-interactive --no-progress sync\n",
                escape_systemd_arg(fav_bin),
                escape_systemd_arg(workspace_dir),
            );
            let timer = format!(
                "[Unit]\n\
                 Description=Run fav sync every {parsed}\n\
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
            let instructions = concat!(
                "Enable and start the user timer (user-level, no root privileges required):\n",
                "  systemctl --user daemon-reload\n",
                "  systemctl --user enable --now fav.timer\n",
                "  systemctl --user status fav.timer",
            )
            .to_owned();
            Ok(GeneratedSchedule {
                files: vec![
                    ScheduleFile {
                        filename: "fav.service".into(),
                        content: service,
                    },
                    ScheduleFile {
                        filename: "fav.timer".into(),
                        content: timer,
                    },
                ],
                instructions,
            })
        }
        ScheduleTarget::Launchd => {
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
                   <string>com.kingwingfly.fav</string>\n\
                   <key>ProgramArguments</key>\n\
                   <array>\n\
                     <string>{bin_xml}</string>\n\
                     <string>-C</string>\n\
                     <string>{dir_xml}</string>\n\
                     <string>--non-interactive</string>\n\
                     <string>--no-progress</string>\n\
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
            let instructions = concat!(
                "Load the LaunchAgent (user-level, no root privileges required):\n",
                "  launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.kingwingfly.fav.plist\n",
                "  launchctl kickstart -k gui/$(id -u)/com.kingwingfly.fav",
            )
            .to_owned();
            Ok(GeneratedSchedule {
                files: vec![ScheduleFile {
                    filename: "com.kingwingfly.fav.plist".into(),
                    content: plist,
                }],
                instructions,
            })
        }
        ScheduleTarget::TaskScheduler => {
            let bin_ps = escape_powershell_single(&fav_bin.to_string_lossy());
            let workspace_raw = workspace_dir.to_string_lossy();
            let dir_ps = escape_powershell_single(&workspace_raw);
            let arg_ps = escape_powershell_single(&format!(
                "-C {} --non-interactive --no-progress sync",
                escape_windows_argv(&workspace_raw),
            ));
            let script = format!(
                "$FavBin = '{bin_ps}'\n\
                 $WorkspaceDir = '{dir_ps}'\n\
                 $Action = New-ScheduledTaskAction -Execute $FavBin -Argument '{arg_ps}'\n\
                 $Trigger = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval ({})\n\
                 $Settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable\n\
                 Register-ScheduledTask -TaskName 'FavSync' -Action $Action -Trigger $Trigger -Settings $Settings -Force\n",
                parsed.powershell_timespan(),
            );
            let instructions = concat!(
                "Register the scheduled task in PowerShell:\n",
                "  powershell -ExecutionPolicy Bypass -File install-fav-task.ps1",
            )
            .to_owned();
            Ok(GeneratedSchedule {
                files: vec![ScheduleFile {
                    filename: "install-fav-task.ps1".into(),
                    content: script,
                }],
                instructions,
            })
        }
    }
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

/// Generate (and optionally install) a user-level schedule configuration, returning
/// human-readable output ready to print.
pub fn schedule(
    target: ScheduleTarget,
    interval: &str,
    workspace_dir: &Path,
    fav_bin: &Path,
    install: bool,
) -> AppResult<String> {
    let generated = generate_schedule(target, interval, workspace_dir, fav_bin)?;
    if install {
        let dest_dir = default_install_dir(target, workspace_dir)?;
        let written = write_schedule_files(&generated, &dest_dir)?;
        let instructions = if target == ScheduleTarget::TaskScheduler
            && let Some(script_path) = written.first()
        {
            let escaped = escape_powershell_single(&script_path.to_string_lossy());
            format!(
                "Register the scheduled task in PowerShell:\n  powershell -ExecutionPolicy Bypass -File '{escaped}'"
            )
        } else {
            generated.instructions
        };
        let mut out = String::new();
        out.push_str("Installed schedule files:\n");
        for path in written {
            out.push_str(&format!("  {}\n", path.display()));
        }
        out.push('\n');
        out.push_str(&instructions);
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
    use crate::error::ExitKind;

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
        assert!(sched.files[0].content.contains(
            "ExecStart=\"/usr/local/bin/fav\" -C \"/home/user/Videos/fav 100%%\" --non-interactive --no-progress sync"
        ));
        assert_eq!(sched.files[1].filename, "fav.timer");
        assert!(sched.files[1].content.contains("OnUnitActiveSec=3h"));
        assert!(sched.files[1].content.contains("Persistent=true"));
        assert!(
            sched
                .instructions
                .contains("\n  systemctl --user daemon-reload")
        );
        assert!(
            sched
                .instructions
                .contains("\n  systemctl --user enable --now fav.timer")
        );
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
        assert!(xml.contains("<string>--non-interactive</string>"));
        assert!(xml.contains("<string>--no-progress</string>"));
        assert!(xml.contains("<string>sync</string>"));
        assert!(sched.instructions.contains("\n  launchctl bootstrap "));
        assert!(!sched.instructions.contains("sudo"));
    }

    #[test]
    fn generate_task_scheduler_script() {
        let sched = generate_schedule(
            ScheduleTarget::TaskScheduler,
            "1d",
            Path::new(r#"C:\Users\O'Brien\$env`dir"quote\Videos"#),
            Path::new(r"C:\bin\fav.exe"),
        )
        .unwrap();
        assert_eq!(sched.files.len(), 1);
        assert_eq!(sched.files[0].filename, "install-fav-task.ps1");
        let ps = &sched.files[0].content;
        assert!(ps.contains(r#"$WorkspaceDir = 'C:\Users\O''Brien\$env`dir"quote\Videos'"#));
        assert!(ps.contains(
            r#"$Action = New-ScheduledTaskAction -Execute $FavBin -Argument '-C "C:\Users\O''Brien\$env`dir\"quote\Videos" --non-interactive --no-progress sync'"#
        ));
        assert!(!ps.contains("\"-C"));
        assert!(ps.contains("New-TimeSpan -Days 1"));
        assert!(ps.contains("Register-ScheduledTask"));
        assert!(
            sched
                .instructions
                .contains("\n  powershell -ExecutionPolicy Bypass -File install-fav-task.ps1")
        );
    }

    #[test]
    fn generate_task_scheduler_windows_argv_trailing_backslashes_and_quotes() {
        assert_eq!(escape_windows_argv(r"D:\"), r#""D:\\""#);
        assert_eq!(escape_windows_argv(r"\\?\D:\"), r#""\\?\D:\\""#);
        assert_eq!(
            escape_windows_argv(r#"C:\dir\"sub\"#),
            r#""C:\dir\\\"sub\\""#
        );

        for (dir, expected_quoted) in [
            (r"D:\", r#""D:\\""#),
            (r"\\?\D:\", r#""\\?\D:\\""#),
            (r#"C:\Users\a\"b\"#, r#""C:\Users\a\\\"b\\""#),
        ] {
            let sched = generate_schedule(
                ScheduleTarget::TaskScheduler,
                "3h",
                Path::new(dir),
                Path::new(r"C:\bin\fav.exe"),
            )
            .unwrap();
            let ps = &sched.files[0].content;
            let dir_ps = escape_powershell_single(dir);
            assert!(
                ps.contains(&format!("$WorkspaceDir = '{dir_ps}'")),
                "missing $WorkspaceDir for {dir}: {ps}"
            );
            assert!(
                ps.contains(&format!(
                    "-Argument '-C {expected_quoted} --non-interactive --no-progress sync'"
                )),
                "unexpected -Argument quoting for {dir}: {ps}"
            );
        }
    }

    #[test]
    fn task_scheduler_install_instructions_use_escaped_installed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("O'Brien $ws");
        fs::create_dir_all(&ws_dir).unwrap();
        let fav_bin = Path::new(r"C:\bin\fav.exe");

        // install == false keeps bare filename
        let dry_out =
            schedule(ScheduleTarget::TaskScheduler, "3h", &ws_dir, fav_bin, false).unwrap();
        assert!(dry_out.contains("powershell -ExecutionPolicy Bypass -File install-fav-task.ps1"));

        // install == true emits the actual installed path, single-quote escaped
        let installed_out =
            schedule(ScheduleTarget::TaskScheduler, "3h", &ws_dir, fav_bin, true).unwrap();
        let expected_path = ws_dir.join(".fav").join("install-fav-task.ps1");
        assert!(expected_path.is_file());
        let escaped = escape_powershell_single(&expected_path.to_string_lossy());
        assert!(
            installed_out.contains(&format!(
                "powershell -ExecutionPolicy Bypass -File '{escaped}'"
            )),
            "expected installed PowerShell path in output, got:\n{installed_out}"
        );
        assert!(escaped.contains("O''Brien $ws"));
    }

    #[test]
    fn schedule_classifies_bad_interval_as_usage_and_io_error_as_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let fav_bin = Path::new("/usr/bin/fav");

        // Invalid --interval -> ExitKind::Usage
        let bad_interval_err =
            schedule(ScheduleTarget::Systemd, "0m", tmp.path(), fav_bin, false).unwrap_err();
        assert_eq!(bad_interval_err.kind(), ExitKind::Usage);

        // Filesystem I/O failure during --install (e.g. `.fav` is a regular file) -> ExitKind::Runtime
        fs::write(tmp.path().join(".fav"), b"not a directory").unwrap();
        let io_err = schedule(
            ScheduleTarget::TaskScheduler,
            "3h",
            tmp.path(),
            fav_bin,
            true,
        )
        .unwrap_err();
        assert_eq!(io_err.kind(), ExitKind::Runtime);
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
}
