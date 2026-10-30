//! The S1's voice, the way `robotd/src/sound.rs` does it: a bank of wavs named by sound tag, one
//! `aplay` per sound on the controller's own speaker (`hw:0,0`, the stock's `pcmC0D0p`).
//!
//! The stock firmware plays its sounds from `/system/audio/sound/*.wav` (100 files, 48 kHz stereo)
//! through this same PCM — there is no bus command for sound, the controller *is* the player. On
//! the bench the bank is symlinks to those files (`s1-push.sh`); phase 2 gives the robot its own.
//! The wheee ride is a one-shot here: the duck's held ride needs a streamed `aplay`, which is
//! phase 2's with the rest of the voice.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use duck_ipc_proto as proto;

pub struct Sound {
    bank: PathBuf,
    device: String,
    child: Option<Child>,
    played: u64,
}

impl Sound {
    pub fn new(bank: PathBuf, device: String) -> Self {
        Self {
            bank,
            device,
            child: None,
            played: 0,
        }
    }

    /// The file a tag plays: `<bank>/<tag>.wav`, the tag spelled as the contract spells it.
    pub fn file_for(&self, tag: proto::SoundTag) -> PathBuf {
        let name = serde_json::to_value(tag)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{tag:?}").to_lowercase());
        self.bank.join(format!("{name}.wav"))
    }

    /// Start the sound; the previous one, if still playing, is cut. `Err` names what is missing.
    pub fn play(&mut self, tag: proto::SoundTag) -> Result<PathBuf, String> {
        let wav = self.file_for(tag);
        if !wav.is_file() {
            return Err(format!("no {} in the sound bank", wav.display()));
        }
        if let Some(mut old) = self.child.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        match Command::new("aplay")
            .args(["-q", "-D", &self.device])
            .arg(&wav)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.child = Some(child);
                self.played += 1;
                Ok(wav)
            }
            Err(e) => Err(format!("aplay: {e}")),
        }
    }

    pub fn played(&self) -> u64 {
        self.played
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tag's wire spelling names the file, and a missing file is a refusal that names it.
    #[test]
    fn a_tag_names_its_file_and_a_missing_one_is_said() {
        let bank = std::env::temp_dir().join(format!("s1d-bank-{}", std::process::id()));
        let mut s = Sound::new(bank.clone(), "hw:0,0".to_owned());
        assert_eq!(s.file_for(proto::SoundTag::Chirp), bank.join("chirp.wav"));
        let err = s.play(proto::SoundTag::Chirp).unwrap_err();
        assert!(err.contains("chirp.wav"), "{err}");
        assert_eq!(s.played(), 0);
    }
}
