//! The weekly check as a Windows scheduled task. It runs this exe with
//! `--check` once a week (or at the next chance if the PC was off), for the
//! signed-in user only, without admin rights. Nothing stays running.

use std::path::Path;

pub const TASK_NAME: &str = "Agent Cleanup weekly check";
/// The argument that runs the check instead of opening the window.
pub const CHECK_ARG: &str = "--check";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    On,
    /// The task runs a different copy of the app (it was moved or renamed).
    OtherCopy,
    /// Task Scheduler couldn't be asked.
    Unknown,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The task definition: every Monday at 10:00, run as soon as possible if
/// that was missed, not on battery, at low priority, stop after two hours.
pub fn task_xml(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Looks for clutter left by AI coding agents and shows a notification if there's something to clean. Turn it off in Agent Cleanup's Settings.</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2026-01-05T10:00:00</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByWeek>
        <DaysOfWeek><Monday /></DaysOfWeek>
        <WeeksInterval>1</WeeksInterval>
      </ScheduleByWeek>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT2H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{}</Command>
      <Arguments>{CHECK_ARG}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        xml_escape(&exe.display().to_string())
    )
}

/// The program a task definition runs.
pub fn task_exe(xml: &str) -> Option<std::path::PathBuf> {
    let xml = xml.replace('\0', "");
    let start = xml.find("<Command>")? + "<Command>".len();
    let end = start + xml[start..].find("</Command>")?;
    let raw = xml[start..end]
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    Some(std::path::PathBuf::from(raw.trim().trim_matches('"')))
}

/// Whether a task definition (as `schtasks /Query /XML` prints it) runs `exe`.
pub fn runs_exe(xml: &str, exe: &Path) -> bool {
    let want = format!(
        "<Command>{}</Command>",
        xml_escape(&exe.display().to_string())
    );
    // Strip NULs in case the output came back as UTF-16.
    xml.replace('\0', "")
        .to_lowercase()
        .contains(&want.to_lowercase())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Output};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn schtasks(args: &[&std::ffi::OsStr]) -> std::io::Result<Output> {
        Command::new("schtasks")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    }

    fn fail(out: &Output) -> anyhow::Error {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::anyhow!(if msg.is_empty() {
            "Task Scheduler refused".to_string()
        } else {
            msg
        })
    }

    pub fn status() -> Status {
        let Ok(exe) = std::env::current_exe() else {
            return Status::Unknown;
        };
        match schtasks(&[
            "/Query".as_ref(),
            "/TN".as_ref(),
            TASK_NAME.as_ref(),
            "/XML".as_ref(),
        ]) {
            Ok(out) if out.status.success() => {
                if runs_exe(&String::from_utf8_lossy(&out.stdout), &exe) {
                    Status::On
                } else {
                    Status::OtherCopy
                }
            }
            Ok(_) => Status::Off,
            Err(_) => Status::Unknown,
        }
    }

    /// If the task runs a copy of the app that no longer exists (the exe was
    /// moved or renamed), points it at this copy. Returns whether it did.
    pub fn repair_if_moved() -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let Ok(out) = schtasks(&[
            "/Query".as_ref(),
            "/TN".as_ref(),
            TASK_NAME.as_ref(),
            "/XML".as_ref(),
        ]) else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        let xml = String::from_utf8_lossy(&out.stdout);
        let old = task_exe(&xml);
        if runs_exe(&xml, &exe) || old.as_deref().is_none_or(Path::exists) {
            return false;
        }
        enable().is_ok()
    }

    pub fn enable() -> anyhow::Result<()> {
        let exe = std::env::current_exe()?;
        // Task Scheduler wants the file in UTF-16 with a byte order mark.
        let mut bytes = vec![0xFF, 0xFE];
        for unit in task_xml(&exe).encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let file = std::env::temp_dir().join("agent-cleanup-task.xml");
        std::fs::write(&file, bytes)?;
        let out = schtasks(&[
            "/Create".as_ref(),
            "/TN".as_ref(),
            TASK_NAME.as_ref(),
            "/XML".as_ref(),
            file.as_os_str(),
            "/F".as_ref(),
        ]);
        let _ = std::fs::remove_file(&file);
        let out = out?;
        if !out.status.success() {
            return Err(fail(&out));
        }
        crate::actions::log_action("turned on the weekly check", &exe);
        Ok(())
    }

    pub fn disable() -> anyhow::Result<()> {
        let out = schtasks(&[
            "/Delete".as_ref(),
            "/TN".as_ref(),
            TASK_NAME.as_ref(),
            "/F".as_ref(),
        ])?;
        if !out.status.success() {
            return Err(fail(&out));
        }
        if let Ok(exe) = std::env::current_exe() {
            crate::actions::log_action("turned off the weekly check", &exe);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn status() -> Status {
        Status::Unknown
    }

    pub fn repair_if_moved() -> bool {
        false
    }

    pub fn enable() -> anyhow::Result<()> {
        anyhow::bail!("the weekly check uses Windows Task Scheduler")
    }

    pub fn disable() -> anyhow::Result<()> {
        anyhow::bail!("the weekly check uses Windows Task Scheduler")
    }
}

pub use imp::{disable, enable, repair_if_moved, status};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_names_the_exe_and_round_trips() {
        let exe = Path::new(r"C:\Tools\A&B\agent-cleanup.exe");
        let xml = task_xml(exe);
        assert!(xml.contains(r"<Command>C:\Tools\A&amp;B\agent-cleanup.exe</Command>"));
        assert!(xml.contains("<Arguments>--check</Arguments>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(runs_exe(&xml, exe));
        // Windows paths are case-insensitive.
        assert!(runs_exe(&xml, Path::new(r"c:\tools\a&b\AGENT-CLEANUP.exe")));
        assert!(!runs_exe(&xml, Path::new(r"C:\Other\agent-cleanup.exe")));
        assert_eq!(task_exe(&xml).as_deref(), Some(exe));
    }
}
