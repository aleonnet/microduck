//! The S1's voice, the way `robotd/src/sound.rs` does it: a bank of wavs named by sound tag, one
//! `aplay` per sound on the controller's own speaker (`hw:0,0`, the stock's `pcmC0D0p`).
//!
//! The stock firmware plays its sounds from `/system/audio/sound/*.wav` (100 files, 48 kHz stereo)
//! through this same PCM — there is no bus command for sound, the controller *is* the player. On
//! the bench the bank is symlinks to those files (`s1-push.sh`); phase 2 gives the robot its own.
//! The wheee is the one wav played as a ride: the pad resends `hold: true` while the trigger is
//! down and `hold: false` once on release, and that hold is a level here as in `robotd` — one
//! start per hold, the release stops it, and a one-shot arriving mid-ride is dropped rather than
//! cutting the ride (holding both triggers is how the pad is used). The duck's streamed ride
//! (start, loop, end segments) is phase 2's with the rest of the voice.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use duck_ipc_proto as proto;

pub struct Sound {
    bank: PathBuf,
    device: String,
    /// The player, `aplay` by name on the robot; a test points it at a stand-in.
    player: PathBuf,
    child: Option<Child>,
    /// What `child` is playing, so a held wheee is not restarted on every tick of the hold.
    playing: Option<proto::SoundTag>,
    played: u64,
}

impl Sound {
    pub fn new(bank: PathBuf, device: String) -> Self {
        Self {
            bank,
            device,
            player: PathBuf::from("aplay"),
            child: None,
            playing: None,
            played: 0,
        }
    }

    /// The same bank through another program (a test's stand-in for `aplay`).
    pub fn with_player(mut self, player: PathBuf) -> Self {
        self.player = player;
        self
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
        self.play_held(tag, None)
    }

    /// `robot.sound` with its `hold`, as the pad sends the wheee: `Some(true)` on every tick the
    /// trigger is down, `Some(false)` once on release (`pad_map::mapper`). The hold is a level, as
    /// `robotd` reads it: a wheee already playing is left alone while the trues keep coming, and
    /// the release stops it instead of playing it once more. Every other tag is one shot — and,
    /// as in `robotd::sound::play`, a one-shot that arrives while the ride plays is dropped: cutting
    /// the ride for a 200 ms chirp would restart it on the next tick of the hold.
    pub fn play_held(
        &mut self,
        tag: proto::SoundTag,
        hold: Option<bool>,
    ) -> Result<PathBuf, String> {
        let wav = self.file_for(tag);
        let riding = self.playing == Some(proto::SoundTag::Wheee)
            && self
                .child
                .as_mut()
                .is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if tag == proto::SoundTag::Wheee {
            match hold {
                Some(false) => {
                    if riding {
                        self.stop();
                    }
                    return Ok(wav);
                }
                Some(true) if riding => return Ok(wav),
                _ => {}
            }
        } else if riding {
            tracing::debug!(?tag, "sound skipped: the wheee ride has the speaker");
            return Ok(wav);
        }
        if !wav.is_file() {
            return Err(format!("no {} in the sound bank", wav.display()));
        }
        self.stop();
        match Command::new(&self.player)
            .args(["-q", "-D", &self.device])
            .arg(&wav)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.child = Some(child);
                self.playing = Some(tag);
                self.played += 1;
                Ok(wav)
            }
            Err(e) => Err(format!("{}: {e}", self.player.display())),
        }
    }

    fn stop(&mut self) {
        if let Some(mut old) = self.child.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        self.playing = None;
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

    /// The pad holds the wheee by resending `hold: true` thirty times a second and releases it
    /// once with `hold: false`: one start per hold, nothing on release (measured before the fix:
    /// six `aplay` spawns for a 150 ms hold). A one-shot tag cuts whatever plays and starts.
    #[test]
    fn a_held_wheee_starts_once_and_the_release_stops_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("s1d-hold-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for tag in ["wheee", "chirp"] {
            std::fs::write(dir.join(format!("{tag}.wav")), b"RIFF").unwrap();
        }
        // The stand-in player: records each start, then plays "forever".
        let log = dir.join("starts.log");
        let player = dir.join("aplay");
        std::fs::write(
            &player,
            format!(
                "#!/bin/sh\necho \"$4\" >> \"{}\"\nexec sleep 30\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&player, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut s = Sound::new(dir.clone(), "hw:0,0".to_owned()).with_player(player);
        let starts = || std::fs::read_to_string(&log).unwrap_or_default();
        // The stand-in is a shell the scheduler runs when it likes: wait for its line with a
        // deadline rather than a fixed sleep (a fixed 200 ms failed 2 runs in 35 under load).
        let settled = |want: usize, wav: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while starts().matches(wav).count() < want && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            // Past the count: a little more time, in case a start is still on its way.
            std::thread::sleep(std::time::Duration::from_millis(100));
            starts().matches(wav).count()
        };
        for _ in 0..5 {
            s.play_held(proto::SoundTag::Wheee, Some(true)).unwrap();
        }
        assert_eq!(settled(1, "wheee.wav"), 1, "{}", starts());
        assert_eq!(s.played(), 1);
        // A chirp during the ride is dropped, as robotd drops it: the ride keeps the speaker.
        // A negative cannot be polled for; 300 ms is ten times what a start took above.
        s.play_held(proto::SoundTag::Chirp, None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            starts().matches("chirp.wav").count(),
            0,
            "the chirp must not cut the ride"
        );
        assert_eq!(s.played(), 1);
        s.play_held(proto::SoundTag::Wheee, Some(true)).unwrap();
        assert_eq!(settled(1, "wheee.wav"), 1, "the ride was not restarted");
        s.play_held(proto::SoundTag::Wheee, Some(false)).unwrap();
        assert!(s.child.is_none(), "the release stops the ride");
        assert_eq!(s.played(), 1, "the release plays nothing");
        s.play_held(proto::SoundTag::Chirp, None).unwrap();
        assert_eq!(settled(1, "chirp.wav"), 1);
        s.play_held(proto::SoundTag::Chirp, None).unwrap();
        assert_eq!(settled(2, "chirp.wav"), 2, "one shot each time");
        s.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
