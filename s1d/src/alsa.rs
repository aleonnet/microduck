//! The mixer as the stock firmware leaves it, restored with ALSA's own tool.
//!
//! The microphone path (MIC2 with its boost, the ADC gains) and the speaker route are mixer
//! controls the stock's `dji_camera` sets at boot and that persist in the codec until power-off.
//! Measured 2026-10-31: with the stock stopped since boot they were still there; after a cold
//! start without the stock they are whatever the kernel's defaults are, and the voice did not
//! reach the recording. `alsactl store` saved the stock's 57 controls on the robot
//! (`deploy/s1/asound.state.stock`); this restores them at start with `alsactl restore`, the
//! mechanism the state was saved with — never a register written by hand (owner's rule).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `alsactl -f <state> restore` through `alsactl` (a test points at a stand-in). The file must
/// exist: a daemon started with a path that is not there is misconfigured, and says so.
pub fn restore_state(state: &Path, alsactl: &Path) -> Result<(), String> {
    if !state.is_file() {
        return Err(format!("no ALSA state file at {}", state.display()));
    }
    let status = Command::new(alsactl)
        .arg("-f")
        .arg(state)
        .arg("restore")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("{}: {e}", alsactl.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("alsactl restore exited with {status}"))
    }
}

/// `alsactl` by name, as the robot's chroot has it.
pub fn default_alsactl() -> PathBuf {
    PathBuf::from("alsactl")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tool is asked exactly `-f <file> restore`; a missing file is refused before any spawn.
    #[test]
    fn restore_asks_alsactl_for_the_file_and_refuses_a_missing_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("s1d-alsa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("asked.log");
        let tool = dir.join("alsactl");
        std::fs::write(
            &tool,
            format!("#!/bin/sh\necho \"$*\" >> \"{}\"\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = dir.join("asound.state");
        let err = restore_state(&state, &tool).unwrap_err();
        assert!(err.contains("no ALSA state file"), "{err}");
        assert!(!log.exists(), "nothing spawned for a missing file");
        std::fs::write(&state, "state.card0 {}\n").unwrap();
        restore_state(&state, &tool).unwrap();
        let asked = std::fs::read_to_string(&log).unwrap();
        assert_eq!(asked.trim(), format!("-f {} restore", state.display()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
