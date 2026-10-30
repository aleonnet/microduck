# S1 bring-up: the RoboMaster S1 as a microduck body

What a DJI RoboMaster S1 does with the microduck daemons. Everything below was measured on the
machine named in each line — the robot, or the Mac that builds for it — and nothing is inferred
from behaviour alone. Where a line says **pending**, nobody has measured it yet and the plan says who
does: the bench stages need the robot plugged in and a person at it.

The design — what of the microduck enters, what is rewritten, and where the body service sits in
place of `robotd` — is phase 2's page; this is the record phase 1 writes as it goes. The plan is
the master plan "S1 ← microduck" (plans directory, 2026-10-29) and the night diary beside it.

## Three words, three levels (fixed 2026-10-30)

The owner lost the map when one word was used for three things, so the vocabulary is fixed here
and every other page follows it:

- **Phase** — only the six of the master plan, one front each: 1 body and senses (the software
  takes the whole hardware), 2 intelligence, 3 media and blaster, 4 system, 5 twin, 6 Space.
- **Stage** — only the twelve bench steps of phase 1 (the table below): 1 capture, 2 backup, 3 chroot,
  4 stop the stock, 5 drive, 6 events and LEDs, 7 camera, 8 audio, 9 app, 10 restore, 11 frames,
  12 inference.
- **Item** — one physical command the body accepts, as `s1d` must send it. Every item belongs to
  phase 1 (owner's rule: everything physical is phase 1) and is proven inside a stage; an item never
  creates a stage or a phase. The inventory of items is the section "Inventory sweep" below.

## What lives where

Two trees, one rule (owner, 2026-10-30): **anything that reproduces a proof is in this repository**,
under `scripts/s1/` — the bench (`bench.sh`) and its self-test against a stand-in `adb`
(`bench-selftest.sh`), the build and push scripts, the strace decoder (`duml-strace.py`), the oracle
that runs DJI's own packer (`dji-oracle.py`), and the end-to-end driver `s1-e2e` (`s1d/src/bin/`),
which plays the pad and the shells for stages 5 and 6 against a fake body or the robot. **The lab, `~/Development/RoboMaster-S1/`, holds only raw matter**:
captures, partition images, the binaries and the Python library pulled from the robot (DJI's code,
never committed), the ONNX Runtime and rootfs builds — each cited here by path and SHA.

## What this settles so far

| | |
|---|---|
| The S1's body speaks DUML/V1 on `/dev/ttyS3` | **yes** — wheels and gimbal moved on 2026-10-29 from frames the lab encoder built; `s1-duml` reproduces those exact bytes (`cargo test -p s1-duml`, 14 tests, 13 of them against `tests/fixtures/*.bin`) |
| The heartbeat `0x3f/0x60` is what arms the chassis | **yes** — without it streamed, nothing moved; the stock firmware sends it at 91.5 Hz (3201 frames in 34.98 s of `captures/ttyS3_drive.strace`) |
| The contract's clients drive this body unchanged | **in code and tests** — `s1d` serves `robot.move/head/stop/enable/init/relax/health/subscribe/mode/policies/flashlight` (`cargo test -p s1d`, 19 tests incl. the deadman against a file body and every robot route of the contract); the bench says whether the robot agrees |
| Our binaries exist for the S1's CPU | **yes** — `cargo s1 -p s1d -p netpadd -p robotctl` → three `ELF 32-bit LSB pie executable, ARM, EABI5` (artefacts below) |
| A Debian armhf userland for the chroot | **built on the Mac** (222 packages; tar and list among the artefacts) — `scripts/s1/build-rootfs.sh`: docker `linux/arm/v7` `debian:bookworm-slim` runs natively here (`uname -m` → `armv7l`, `ldd --version` → `GLIBC 2.36-9+deb12u14`) |
| ONNX Runtime for armv7 | **built** — `scripts/s1/build-onnxruntime.sh` (v1.28.0, cross per onnxruntime.ai's "Cross compiling on Linux"): `libonnxruntime.so.1.28.0`, 19179636 bytes, `ELF 32-bit LSB shared object, ARM, EABI5`; the first attempt died on `-Werror=restrict` (GCC 12, `char_traits`, 32-bit) and the script now passes cmake `--compile-no-warning-as-error`, which is what ORT's own `build.py` does |
| The detector's CPU path runs over that library | **on the Mac, on a synthetic frame** — `s1-infer` (the ONNX measurement `duck-bench` does not make: that binary is the NPU path, and the S1 has no NPU) over `duck_detect.onnx` from the Hub: p50 13.1 ms at 320×320 on this Mac with the macOS ORT 1.28.0 release; the S1's own number is stage 12 |
| Everything the robot has to answer | **pending** — stages 1–12 below |

## The bench, stage by stage

Run from the Mac with `scripts/s1/bench.sh <stage>`; `S1_HOST` names the robot (root adb on
5555, enabled from the app's Lab — `pierluigizagaria/robomaster-s1-hacks`). Artefacts land under
`~/Development/RoboMaster-S1/out/<date>-poc2/<stage>/` and are cited here as `artefato: <path>
sha256=<hex>` lines; `bench.sh verify-record docs/project/s1-bringup.md` checks every one.

Order is not optional: capture **before** stopping anything (the lesson of the first PoC, which
killed the stock services before learning how they bring up audio and camera), back up before any
write, prove execution before pushing, free the UART before driving. Wheels off the ground for
stages 5, 6 and 8.

| # | stage | what it proves | status |
|---|---|---|---|
| 1 | `capture` | cpuinfo, partitions, mounts, `getenforce`, the UART's baud (`stty`), and `strace` of `dji_hdvt_uav` and `dji_camera` while the app is used: LED effects, armour hits, button, audio and camera init, `top` during person recognition | **done 2026-10-30** (below) |
| 2 | `backup` | every partition dumped, SHA on robot == SHA on Mac | **done 2026-10-30** (below): 14 on the card, 11 pulled, SHA equal |
| 3 | `exec` | a binary of ours runs on the robot (`/data`, or `setenforce 0` by the owner, or the microSD); the chroot answers `uname -a` and `ldd --version` | **done 2026-10-30** (below) — gate passed |
| 4 | `stop` | `hdvt_uav` and `monitor` stopped; nobody holds `ttyS3` | **done 2026-10-30** (below) |
| 5 | `drive` + `items` | `s1d` + `netpadd` + `robotctl health` in the chroot; `s1-e2e robot` from the Mac: the pad over UDP into `netpadd` (wheels, turn, strafe, gimbal, lights, sounds, arm/disarm), the S1 list through `s1ctl --tcp`, telemetry | **wheels and gimbal done 2026-10-30** (below, owner's witness); **every item pre-tested on the Mac** (`s1-e2e fake`, 40 of 40, below); the robot session runs the same program minus the two wire-only items (a test holds the two lists equal); telemetry still refused by the controller — open |
| 6 | `events` | `robotctl flashlight` lights the LEDs; every effect of the real `0x3f/0x32`; armour hits, IR and the button as `s1.event` | pre-tested on the Mac (events fed to the fake body reach `s1ctl events`; the button through a FIFO); pending on the robot |
| 7 | `camera` | `v4l2-ctl` formats and one NV12 frame → PNG | pending |
| 8 | `audio` | mic as a 60 s stream through `arecord`; `aplay` while driving, CPU sampled | pending |
| 9 | `app` | what the DJI app does with the stock stopped | pending (informative) |
| 10 | `restore` | reboot brings the stock back; the app drives | pending |
| 11 | `frames` | `gst-launch-1.0 v4l2src` — `mediad`'s element — at 720p and 360p: fps and CPU | pending — **gates the port** |
| 12 | `infer` | `s1-infer` (the detector's ONNX path) on the S1 with `libonnxruntime` armv7 and on the Mac over the same frames: ms/frame on both tiers | pending on the robot; the Mac tier ran tonight on a synthetic frame (below) — **gates the port** (one tier suffices) |

The `items` stage is `s1-e2e robot` alone, with the daemons already up; `e2e` is the same program
with no robot (`s1-e2e fake`), the pre-test every stage-5 and stage-6 item passes before the robot
is plugged in; `bench-selftest.sh` runs every other stage against a stand-in `adb` and checks what
each one asks of the robot.

Viability of the port = stages 3, 5, 11 and 12 proven. 6, 7 and 8 are what phase 2 (intelligence)
consumes; 9 informs; 2 is the prerequisite of phase 4's autostart, not of this phase.

## Measured on the Mac, 2026-10-30 (night)

```
cargo test -p s1-duml -p s1d            → 33 passed, 0 failed (14 + 19)
RUSTFLAGS="-D warnings" cargo clippy -p s1-duml -p s1d --all-targets   → clean
cargo s1 -p s1d -p netpadd -p robotctl  → Finished release, 16.98 s
file target/armv7-unknown-linux-gnueabihf/release/s1d
   ELF 32-bit LSB pie executable, ARM, EABI5 version 1 (SYSV), dynamically linked,
   interpreter /lib/ld-linux-armhf.so.3, for GNU/Linux 2.0.0, stripped
docker run --rm --platform linux/arm/v7 debian:bookworm-slim sh -c 'uname -m; ldd --version | head -1'
   armv7l / ldd (Debian GLIBC 2.36-9+deb12u14) 2.36
```

Refutations run with the defect planted and the file restored (sha256 equal before and after):
the CRC16 seed set to `0x0000` fails `real_heartbeat_crcs` (`left: 19680 right: 20899`); one
byte of the real DDS push flipped fails with `BodyCrc { computed: 24994, found: 9918 }`; the
deadman's age check replaced by `false` fails `the_deadman_zeroes_the_wheels_and_keeps_the_heartbeat`
with non-zero wheels after the deadman.

The cold reader's round on the night's diff (REJECTED, then fixed): eleven robot routes fell to
METHOD_NOT_FOUND (now each answers or refuses with a reason; a test walks every robot route of the
contract); `robot.flashlight` on/off did not update what `toggle` inverts (fixed; the test follows
the pad's HOME-after-`robotctl flashlight on` scenario); `bench.sh backup` compared SHA lists in
two formats (normalised); `bench.sh infer` pushed a binary that does not exist (now `s1-infer`);
the public `Frame::decode` panicked on a header declaring fewer than 13 bytes (now `BadLength`);
the telemetry subscription is re-asked after the controller falls silent, not only before its
first push; `strace` in stage 1 is bounded to 70 s; `dd bs=1M`; `robotctl health`'s exit code no
longer aborts stage 5; the daemons start under `nohup`.

```
ORT_DYLIB_PATH=…/onnxruntime-osx-arm64-1.28.0/lib/libonnxruntime.1.28.0.dylib target/release/s1-infer \
   --model duck_detect.onnx --frames frames-synthetic --hz 0 --passes 10
   model 320×320×3 · latency p50 13.1 ms · p95 13.5 ms · max 13.5 ms · 75.56 fps flat out
   detections 0 in 1 frames   (a gradient, not a duck — this run times the path, it does not test the model)
```

Artefacts of the night (the bench adds its own lines per stage):

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/build/s1d sha256=d1acd265a0509ea6f8b47dbfbbc7f2ec95da1dd3d1007729ec4f2f79018f6bdc
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/build/netpadd sha256=b24219da48363367738c3b2fdbceae452d12b5c3e75a4bbc38dc9ea26dfff53d
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/build/robotctl sha256=c20981e069ebbd4e4f983874f0cf203504c0a7cfef072f8e8fe3a5f515edadb7
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/build/file.txt sha256=384d23cc9dc4527ed46bdfdf7caba728da344fe3447f00d8a67ff74369b70784

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/build/s1-infer sha256=b2a4510391320d1fed0a9960c8906eff04f799b1b475135c5bb1bc49829240ce
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/onnxruntime/libonnxruntime.so.1.28.0 sha256=3b114ea4d42caf5f61dcb9c1b3019f2bbd94ad11cc4cbfbb8b12bd928b4d866d
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/onnxruntime/build.log sha256=95c2d2741104bbaf54e841be95bd229672eef41f9fc72131e3476a25603aaff2
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/duck_detect.onnx sha256=2e34657a655a15221f93f6149730875274da4fb1b1c67dc883cde769fec35c30
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/frames-synthetic/gradient_1280x720.png sha256=2ad9b3c8ebb74753075db68fe5de40c419706ad88bc8a6dc5147bdb3c642c6db
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/infer-mac-synthetic.txt sha256=74b3eafad18d7a8261a2efa05512529a4fb31253a344f00fada189ba6dbe6ec2

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/rootfs/rootfs-bookworm-armhf.tar sha256=19ff2018f22396f710e97fea7ef3c9c70f539c108cf8316e078628d30e3abdf8
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/rootfs/rootfs-bookworm-armhf.packages sha256=531c51d612a103205071fd663416f1f9995c808f097ad7c84fe960c8c1f660d4


## Pre-test on the Mac, 2026-10-30 (night) — phase 1 through the duck's mechanism, no robot

The owner's order: nothing goes back to the robot until the whole of phase 1 has been exercised by
the same mechanism a duck is driven with, and every item is validated in one end-to-end run. The
mechanism is `pad_map::wire` datagrams (what `duckctl udp-pad` sends) into `netpadd`, `robotctl`
and `s1ctl` on the robot socket, and `s1d` on the wire; `s1-e2e fake` starts `s1d --fake` (frames
to a file, pushes from a file — `--fake-in`), `netpadd`, and runs the real `robotctl` and `s1ctl`
binaries beside them, then judges every item on the bytes the loop wrote: against DJI's own
library's frames (`dji-oracle.py`, byte for byte) or the frames captured off the robot. It is also a
test (`s1d/tests/e2e.rs`, so `cargo test -p s1d` runs it) and a bench stage (`bench.sh e2e`).

```
cargo test -p s1-duml -p s1d     → 70 passed, 0 failed (33 + 37; the e2e test builds netpadd and robotctl itself, 23 s)
scripts/s1/bench.sh e2e          → 40 of 40 items passed · socket round trip over 15 calls: min 0.1 ms · p50 0.1 ms · max 0.5 ms
scripts/s1/e2e-refute.sh         → the planted byte was refused; the bench can fail (source restored, sha256 equal)
scripts/s1/bench-selftest.sh     → every stage ran and asked what it must (43 artefact lines verified)
RUSTFLAGS="-D warnings" cargo clippy -p s1-duml -p s1d --all-targets → clean · cargo fmt --check → clean
```

The 40 items, in the order the robot session repeats them (`bench.sh drive` runs `s1-e2e robot`:
the same names, the same calls, the real `s1ctl --tcp`, minus the two wire-only checks — the
test `items_are_the_same_program` in `s1-e2e.rs` holds the two lists equal; a person at the robot
is the witness there, and only Enter or `y` passes an item):

| through | items | judged on |
|---|---|---|
| `robotctl health --json` | the loop healthy | the answer |
| the pad (UDP → `netpadd` → `robot.*`) | Start arms (five work-mode-1 frames, then the heartbeat); left stick forward (all four wheels, `[fr, −fl, −bl, br]` equal); right stick turns (sides opposed); left stick strafes; sticks centred (wheels zero); D-pad right + right stick (a gimbal frame with yaw and pitch both non-zero, read back from the frame); stick centred (the last gimbal frame is `(0, 0)`); Home twice (LEDs white solid, then off); RT and LT (one `aplay chirp.wav`, and **one** `wheee.wav` for a 300 ms hold — the hold is a level); Start disarms (no heartbeat for 300 ms) | the wire, and `aplay`'s stand-in |
| `s1ctl` and the socket | version; `robot.enable`; F16 wheels (`[100, −100, −100, 100]`, zero after `robot.stop`); **the deadman** (one `robot.move` and silence: 20–70 moving wheel frames, then zero, no stop sent); F1 follow gimbal_lead (gimbal work mode 0, stick overlay 1, the twist as `0x3f/0x22` with no wheel speeds), chassis_lead (work mode 2), F7 velocity frame gimbal (overlay 2), F4 FPV (work mode 1), free; F2 recenter; F3 speed 30/−15 then stop; F5 sleep, wake; F6 goto 1 −0.5 −90 then `--stop`; F15 chassis work mode 2 then 1 as raw frames; F10 lights solid red, breath blue, flash green, marquee white, off (effects `[1, 2, 3, 4, 0]` on the wire); F9 blaster LED; `robotctl flashlight on --color red`, off; F11 armour 1.0; F12 PWM 500 ‰ / 1 kHz; F17 blaster infrared, gel refused naming the key; volume 50 (`amixer -q -c 0 sset Speaker 50%`); `robot.sound greet` and `robotctl quack` (`aplay greet.wav`, `chirp.wav`); a raw frame | the wire: the oracle hex of each command (`dji_oracle.rs`); where a builder is used instead, that builder is itself held to the oracle or to a real fixture |
| telemetry | the subscription on the wire (add-node byte-identical to the stock's `dds_add_node_stock_real.bin`, node reset, nine subjects under msg `0x5d` with flags 3); a push of ours fed to the body → `s1ctl telemetry` reads battery 77 %, gimbal `[90, −15]`, picked up, x 1.5 m, wheel 123 rpm, the controller's answer `0x00` logged as accepted, and the push **acked with the stock's exact frame, the push's sequence included** (`dds_push_ack_stock_real.bin`; `fake_body.rs` holds the same for the stock's push and ours); `robotctl monitor --json` → `robot.state` head yaw 1.5708 rad, odometry x 1.5, picked up | `s1ctl`, `robotctl`, the wire |
| events | an armour hit on plate 3, an infrared hit and a button press (an `input_event` through a FIFO) → three `s1.event` notifications printed by `s1ctl events` | `s1ctl` |
| the end | `robot.enable off` (no heartbeat after); every byte on the wire decodes as whole frames (4112 frames, 0 dropped) | the wire |

Defects the pre-test found, each fixed and held by a test or the run itself:

1. **The pad's sounds never played on the S1.** `pad-map` sends `robot.sound` as a notification
   (no id) on a trigger's edge; `s1d`'s notification path applied only move and head. The run's
   first pass: `pad RT quacks, LT rides  FAIL  aplay log: ""`. Fixed (`ipc::apply_notification`).
2. **The left stick's sideways travel did nothing.** `[pad_roller]`'s defaults are the prototype
   roller's "no strafe" (`vy_min = vy_max = 0`); a mecanum base strafes. First pass:
   `pad left stick strafe  FAIL  0 moving wheel frames`. Fixed in `deploy/s1/robotd.toml`
   (`[pad_roller]` with `vy ±0.5`, `vyaw ±1.0` — bench values, not the S1's maxima), held by
   `tests/params.rs`.
3. **`robot.sound` on the robot answered `aplay: No such file or directory`.** The daemon in the
   chroot inherits Android's PATH (`/sbin:/system/bin:/system/xbin`, read from
   `/proc/<pid>/environ` on the robot). `bench.sh` now enters the chroot with Debian's PATH
   (`/usr/bin/env PATH=…`), and `bench-selftest.sh` checks that line is what the stage asks.
4. **The sound bank was never created on the robot.** `s1-push.sh`'s loop used the Android shell's
   `ln`, which is not on the adb shell's path; it created nothing and said nothing. Now
   `busybox ln`, and the script fails unless seven entries exist.
5. **Pushes were not acknowledged.** The stock acks every `0x48/0x08` push (1849 in 70 s); the
   controller has never been seen pushing to a subscriber that does not. `s1d` now answers each
   push with the stock's frame (`dds::push_ack`, fixture `dds_push_ack_stock_real.bin`, byte for
   byte). Whether that is what the controller was waiting for is the robot session's first answer.
6. **A held wheee restarted thirty times a second** (the cold reader's round on the first cut).
   `pad-map` resends `robot.sound {wheee, hold: true}` on every tick the left trigger is down and
   `hold: false` once on release; `s1d` played each one, cutting the last (six `aplay` spawns for
   a 150 ms hold, measured). Now the hold is a level, as `robotd` reads it (`Sound::play_held`):
   one start per hold, the release stops it; held by a unit test with a stand-in player and by the
   pad item, which counts exactly one `wheee.wav` for a 300 ms hold.

The same round took four things out of the bench itself: the pad-gimbal judge passed on any gimbal
frame (every one carries fixed non-zero bytes) and now reads yaw and pitch back; the robot-mode
witness passed anything that did not contain "no" and now passes only Enter or `y`; the robot mode
had 32 items to the fake's 38 and its own JSON client — now the same program (`s1ctl --tcp`, the
missing items, a test over both lists); the deadman and F15 were not exercised end to end, and
are. `bench-selftest.sh` runs `s1-e2e robot` against port 1 (`S1_TCP`, `S1_UDP`), which refuses,
so a forward left on 9000 by a robot session cannot make the self-test drive a robot; `bench.sh
restore` removes that forward. `bench.sh drive` passes `--volume-control` only when
`S1_VOLUME_CONTROL` names the speaker's control (stage 8's dump), so the volume item is refused
by name until then, not reported as passed.

One choice the pre-test cannot settle: `s1.blaster` addresses the blaster host (`0x17`), while
DJI's library sends the fire command to the stock controller (`0x09`), which is stopped under us.
The frame's bytes are the library's; the receiver is the bench's to confirm, and the robot session's
F17 item is where it is decided.

Refutation, executable (`scripts/s1/e2e-refute.sh`, its output among the artefacts): one byte of
F2's expected hex planted →
`F2 gimbal recenter  FAIL   wire lacks 551604fcc9040000403fb2070805640000006400bfa1; had ["3f/b2"]`,
`0 of 1 items passed — FAILED`; the source restored, sha256 equal before and after. Items 1, 2 and
6 above are the run's own refutations: each failed with the text quoted before its fix.

What the pre-test cannot do, and the robot session does, in this order: `exec` (remount `/system`
into the chroot), `stop`, `drive` (the 38 items, wheels off the ground; the DDS answer codes with
the push ack in place, and once more after `robot.enable` so the subscription is asked while the
heartbeat streams), `events`, `camera`, `audio`, `frames`, `infer`, `restore` — then `capture` again
with the stock alive for the button-press power-off (`dji_sys`, the frame behind `s1.power`) and the
speaker-volume device. The Coral stays phase 2's (owner's decision, 2026-10-30).

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/e2e_fake.txt sha256=521e79ecef012f133288b276af3c7528ae2ad06012258c93298a385bb45b12a8
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/work/report.txt sha256=a6c9a7ba6cd55d93b25e9a785f5201f026ca48e1cac98ef4ff0381ae1bf739c5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/work/wire.bin sha256=f3c8887d6035284966f079ff84ce4985c136a0606015e016b6f1e6aba7b95bd2
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/refute.txt sha256=f0e510cff90677fa6a8b47cebe5dd5c0357b5da1aab9247e6a42a8b42ba022f9

## Bench, 2026-10-30 morning — stage 1 (stock alive)

Root adb opened through the app's Lab (`robomaster-s1-hacks`, `enable_root_adb.py` → `S1_ADB_READY`);
`adb shell id` → `uid=0(root)`, `ro.product.model` → `L1860`, Android 4.4.4, **`getenforce` →
`Disabled`** (the PoC's "SELinux blocks /data" belief was wrong; stage 3 says what does).

Measured:

- **Baud of `/dev/ttyS3`: 921600** (`busybox stty -F /dev/ttyS3 -a` → `speed 921600 baud`). `S1_BAUD=921600`.
- **The LED command is `0x3f/0x32` to host `0x18`**, thirteen bytes `effect, led_mask u16, r g b,
  repeat, t1 u16, t2 u16, comp_mask u16` — three frames captured while the app changed colours
  (`hdvt_uav.strace`, `write(16, …)`): solid teal `01 ff00 00 7f 46 00 0000 0000 3f00` and white
  breath `02 ff00 ff ff ff 00 f401 f401 3f00`. The SDK's `0x3f/0x33` never appears. `s1-duml`'s
  `system_led` now reproduces both byte for byte (`tests/fixtures/led_stock_{solid,breath}_real.bin`).
- Stock traffic in 70 s: `0x04/0x69` 6093 and heartbeat `0x3f/0x60` 6084 writes (~87 Hz each),
  1849 acks of DDS pushes (`0x48/0x08`, attri `0xa0`, data `01` — **the controller acks every push**;
  `s1d` does not yet), `0x0a/0x53` 672 (~10 Hz), `0x04/0x4c` 43, `0x03/0xd7` 27, `0x00/0x58` 14.
  No `0x3f/0x02` armour, `0x3f/0x10` IR or `0x3f/0x51` blaster frames were written by the controller
  in this window (they would be *reads*, and `strace -s 64` truncates reads mid-stream: 4324
  truncated payloads).
- **A microSD is in the robot**: `mmcblk1p1` 29.8 GB vfat at `/sdcard`. eMMC 3.8 GB: `system` 256 MB,
  `userdata` 1.1 GB, `blackbox` 1.9 GB, `cache` 256 MB, `recovery`/`normal` 16 MB, `vendor`/`amt`
  64 MB. `/data` 679 MB free, `ext4 rw` (no `noexec` flag in `/proc/mounts`).
- 5 cores ARMv7 rev 5, NEON, VFPv4 (`/proc/cpuinfo`). `dji_camera` was streaming (`VIDIOC_DQBUF`).
- Fixed on the way: the S1's busybox 1.25.1 `timeout` wants `-t SECS` (the bare form exits 127);
  `bench.sh` uses it in stages 1, 5 and 6. Backup staging moved to `/sdcard/bk`.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/_system_xbin_busybox_stty_-F_dev_ttyS3_-a.txt sha256=10858ddaed015132b5902e90bd13f52a55d43175680d44c2cad112889dcb9a39
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/cat_proc_cpuinfo.txt sha256=af48bb92b627987ced943faa95d05373aca6a0b91da966686308418459056a0a
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/cat_proc_mounts.txt sha256=b6aa9da3e0ee0741c2c7486b1cffa51233166ff3ed5c1ad416c3bc0f8a9fc80d
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/cat_proc_partitions.txt sha256=1607a47bfc9f7ac309218559e294106ea15462ac10e05a4ca9cd3a66384d628b
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/cmd.txt sha256=70dcb6ca7bdb5eb9ea3559a92172baddd406f66da5a910162a15509a02f409f5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/df.txt sha256=926eef759f3f7b84a115f4484a202aa9b789ff19684df90e9a97b53962d9ce4c
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/dji_camera.strace sha256=87fdfa49843e68d3d71c1f9bce8d3702d790ad3982a476506f7c040bbfb24f3b
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/getenforce.txt sha256=1535895fea3509918515479905fb814e80657ea5db44bcd7bbdf6ffbde370de2
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/getprop.txt sha256=9d9c50c6d93da4f3a40d1b9f75761bc8027e2d17091f2c97c9fa0471d38cdfb4
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/hdvt_uav.strace sha256=f4af06fd573a4e37e849ca556443f1db2a6e65c9d567cc978a1f154b0fa924dc
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/ls_-l_dev_block_platform_by-name.txt sha256=c7fbf3de2a68018357f375d2478d0ffe38dd26ef1af067f2f63494159ec7b04e
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/ls_-l_dev_input_dev_video_dev_snd.txt sha256=d01c90f276822fd84c39c9bb2f720bb701b7d3023edc52959a29f1df710e484d
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/ps.txt sha256=ba896e5a5f6f9f7cdc56ab5a13268d9db9814c99abdd1ceda37e0b76228a2acd
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/top_before.txt sha256=cb3a80069c80834eeb59a86b52578e9880610295104c98bbc4d345644346da2b
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/top_vision.txt sha256=abda4d6fb67b6d63551a1c42c119e8f08114ed0820f9f6fc633889d521ded8c7

## Bench, 2026-10-30 morning — stage 2 (backup)

`bench.sh backup` with the staging on the card (`S1_STAGE=/sdcard/bk`): all **14 partitions of the
eMMC** dumped by `dd` between 09:00 and 09:11 (3.75 GB on the vfat card, `busybox ls -l /sdcard/bk`),
`sha256sum` run on the robot over all 14 (`backup/SHA256SUMS.robot`), then the images that fit
pulled to the Mac at ~1.4 MB/s and hashed again (`backup/SHA256SUMS`): **the 11 pulled images hash
the same on both sides** (`bench.sh` prints `SHA robot == Mac (pulled set)`; 452 MB on the Mac).
`blackbox` (1.9 GB), `userdata` (1.1 GB) and `cache` (256 MB) stay on the card with their robot-side
hash on record; they are pulled later in the background, not on the bench's clock.

Observed, not interpreted: `ddr`, `gpt` and `panic` are 4 MB each and read back as all zeros
(the three share one SHA, `bb9f8df6…`). Restoring them from these images changes nothing; where the
controller keeps its partition table is not settled by this dump. Restoring any other partition is
`dd` of the image from a root shell; the images of `userdata`, `cache` and `blackbox` are live
snapshots of a running system, not a quiesced one.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/SHA256SUMS sha256=d8987f72046b0381f87cdcb3df7d86518e5d2d336b072aee808c5d760a60c0ae
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/SHA256SUMS.robot sha256=068c0a308fbc2b012ab66ea8eb2ca004b21080cf044561a3138ece4336c05db3
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/amt.img sha256=568feb8315a9160bd7e4da5ba2892d281aaca85dac78f207d5970362f4089cc0
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/ddr.img sha256=bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/env.img sha256=e79002df36d720eb8ed90b352db184e0f824b3d3bfc0ff8aac728e4b4eb91c64
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/factory.img sha256=8e92781d32dba7e4d3e1a5151bf85ee9b19b03b45d43e889931d805171617def
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/factory_out.img sha256=0382c952a2e9fcba6f2230617772f886c0c91fdc11a342fd71cb39c38bedbd62
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/gpt.img sha256=bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/normal.img sha256=0398e49b594bf290daa473551dd387422d374959684daaf97fce971203c0d274
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/panic.img sha256=bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/recovery.img sha256=38737e740abb46ae49848aa6928957d69f14f0f41c3a6820b8735aa3180c952d
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/system.img sha256=10cc3cf264cc013d1c5bf2189a2aa86bfe3c58f522d97c4687480cb7232d5c02
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/bk/vendor.img sha256=73faeae59cae04b3ab169a13e4110395ddc0cf20cc5fd42ad0003da84fbc7123

## Bench, 2026-10-30 — stages 3, 4 and 5

**Stage 3, execution and chroot — proven.** `busybox` copied to `/data` runs (`EXEC_IN_DATA_OK`; the
first probe copied it as `bb` and busybox, a multi-call binary, answered `bb: applet not found` —
that message *is* execution from `/data`, and the probe now copies it under its own name). The
Debian bookworm armhf rootfs (177 MB tar, 171 MB extracted; `/data` had 658 MB free) was pushed at
0.6 MB/s in 260 s and extracted to `/data/debian`; `/dev`, `/proc`, `/sys` bind-mounted; the Android
shell has no `chroot` or `head`, both are busybox applets here. `busybox chroot /data/debian`:
`CHROOT_OK`, `Linux localhost 3.10.62 #1 SMP PREEMPT Thu Oct 27 03:34:03 CST 2022 armv7l`,
`ldd (Debian GLIBC 2.36-9+deb12u14) 2.36`, Debian `12.15`.

**Stage 4, free the UART — proven.** `stop dji_hdvt_uav; stop dji_monitor` + kill →
`hdvt_uav_stopped`; the scan of every `/proc/*/fd` found nobody on `ttyS3`. `dji_camera`,
`dji_vision`, `dji_sys`, `dji_network`, `dji_sw_uav`, `dji_blackbox` keep running.

**Stage 5, drive — wheels and gimbal proven, telemetry open.** `s1d` and `netpadd` run in the
chroot (they only survive the adb shell as `(setsid nohup … </dev/null >log 2>&1 &); sleep 1` —
measured: plain `nohup … &` left an empty log, and `setsid` without the sleep lost the race once).
`robotctl health --json` in the chroot: `healthy: true`, 99.93 Hz, 0 missed, 0 bus errors;
`robot.mode` → `roller`, `robot.modelApi` → 2. Without a gamepad the Mac drives the JSON-RPC
socket through `adb forward tcp:9000 localfilesystem:/data/debian/run/robotd.sock` (`drive.py`):
`robot.enable` → `armed`; 30 × `robot.move` forward 0.2 m/s, 20 × yaw 0.6 rad/s, 20 × strafe
0.2 m/s, 50 × `robot.head`, every one `accepted`; `robot.enable off` → `disarmed`; the owner, wheels
in the air: **"funcionou"** — wheels and gimbal obeyed. Health `healthy` before and after.

Telemetry, three defects found and two fixed on the bench, one open:
1. The motion controller keeps pushing the **stock firmware's** subscriptions after `dji_hdvt_uav`
   is stopped — msg id 1 at 50 Hz (161 B of subjects) and msg id 0 at 1 Hz (54 B) — and `s1d` used
   msg id 1 too, so it decoded the stock's subjects with our layout (gyro in the 10³² range,
   `monitor_before_fix.log`). Fixed: own msg id `0x5d`, a push is ours only at exactly
   `DDS_PUSH_LEN` bytes, foreign pushes counted; the subscription is sent armed or not.
2. The controller then ignored our subscription outright. Running the stock `dji_hdvt_uav` under
   `strace` for 10 s (`hdvt_boot.strace`) shows what it sends on start: `0x48/0x01` register node 9
   (`09 00 00 00 03`), then `0x48/0x03` subscribe — both with attribute `0x40`, the SDK's
   `need_ack = 2` ("ack when finished"), a two-bit field our frame modelled as one bit. Fixed:
   `Ack::{None, Now, WhenDone}`, `dds::add_node`, both requests `WhenDone`; the two stock frames
   are fixtures reproduced byte for byte. The controller now **answers** both requests.
3. Open: the answers are refusals — add-node returns `02 03` (the SDK accepts only `0x00` and
   `0x50`), subscribe returns `21 03 00 5d 00…`. Next: `0x48/0x02` node reset before registering,
   as the SDK's `ProtoSubNodeReset` allows; measured on the next power-up.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/exec/exec.txt sha256=3d38161894241b029aa2db218b42271da30344015b1e236c1e0f6cf1364d96a0
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/stop/stop.txt sha256=93993a72aca42aaa5d68f7f5908a965278cb4465616b8ad9b55f1e7cef566425
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/stop/owners.txt sha256=dfa3ab767dc85cd9b759818b95b1d7df1a7ec15dd211b83eb3ca97481c099705
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/health.json sha256=d729d1d41ceb614d05f9d2c7ad0a5a8a4c7a8065de0335e53123935a52e031d9
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/move.log sha256=3d3ef2f5bb7664844e21a33e67e6c1b789ef03bbceb0caf06aa49d65e16f1f6a
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/drive.py sha256=543331d05db8d47ae0e3fdbf9235bcfe9d8a1e1f2a6634ec9fdbef36886a52da
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/hdvt_boot.strace sha256=41ca4f9d1f03599dc419096895f1df7a2248240b82a49114c44bb6eb62b34ddb
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/monitor_before_fix.log sha256=324ad59be5e3e6f11ef6036dd585230b2a5d077e77b0fcf5c53d475bcf263432
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/s1d_trace_after_ack_fix.log sha256=390cfee9e4fa22963ed40ec86e75f0b533a13c37cb6bbab98c280c65543c86fa
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/mirror-check-20261030T122913Z.txt sha256=b84188f7ca61a2eaaf4db2e1776d8a261d3c9caa69eabb411e032b9fd31e1797

## Inventory sweep against the official SDK, 2026-10-30

The master plan's body inventory was built from captures and proroklab, not from a whole reading
of the primary sources; the owner found the hole (the app's "chassis follows gimbal" button) and
ruled: **every physical command belongs to phase 1**, intelligence to phase 2. This section is the
whole reading: `dji-sdk/RoboMaster-SDK` `src/robomaster/{protocol,robot,chassis,gimbal,led,blaster,
armor,vision,battery,camera,dds}.py` — 61 `Proto*` classes, every public method of the body modules,
18 DDS subjects. Proven already: wheels (`0x3f/0x20`, `0x21`), heartbeat `0x3f/0x60`, gimbal by
angle `0x3f/0xb0`, system LED (real id `0x3f/0x32`), armour `0x3f/0x02`, IR `0x3f/0x10`, the button,
DDS battery/attitude/odometry/IMU/esc/sa_status.

Physical commands the inventory lacked — proposed for phase 1, awaiting the owner's ratification:

| # | what the robot does | command (SDK name) | request layout | in the code |
|---|---|---|---|---|
| F1 | chassis follows the gimbal / gimbal follows the chassis / free — the "twist" | `0x3f/0x46` goes to the stock controller; on the bus the library does it with chassis work mode 1 + the follow twist `0x3f/0x22` (gimbal lead) or gimbal work mode 2 `0x04/0x4c` (chassis lead) | follow: vx, vy, yaw offset as floats | `s1.mode` + `chassis_follow` / `gimbal_workmode`, oracle-exact; bench proves the motion |
| F2 | recentre the gimbal | `gimbal.recenter` `0x3f/0xb2` | action_id, ctrl\|freq<<2, valid yaw/roll/pitch bits, 3× u16 speed | absent (listed in the plan, never built) |
| F3 | gimbal by rate, °/s, continuous | `gimbal.drive_speed` `0x04/0x0c` | yaw, roll, pitch i16 + ctrl byte | absent; what phase 2's look/follow needs |
| F4 | gimbal work mode | `0x04/0x4c` (the stock writes it 43× in 70 s) | workmode, recenter | absent; semantics to capture |
| F5 | gimbal sleep / wake (folds at power-off) | `gimbal.suspend/resume` `0x04/0x0d` | u16 order code `0x2ab5` / `0x7ef2` | absent; needed at shutdown and boot |
| F6 | move a distance and stop, loop closed by the controller | `chassis.move` `0x3f/0x25`; progress push `0x3f/0x2a` | action_id, ctrl\|freq, modes, x y z i16, vel_xy, omega_max i16 | absent; the base of choreography and patrol |
| F7 | velocity frame: body or gimbal heading | `chassis.stick_overlay` `0x3f/0x28` | 1 byte: 0 off, 1 body, 2 gimbal | absent |
| F8 | built-in sounds: attack, shoot, scanning, recognised, countdown, musical notes 1C…3B | **no bus command**: the library sends `0x3f/0x1a` to the stock controller, which plays `/system/audio/sound/*.wav` (100 files, 48 kHz stereo) on its own PCM | — | `robot.sound` plays the same files through `aplay` (`s1d --sounds`); the notes are an instrument for the carnival |
| F9 | blaster LED (trail light) | `blaster.set_led` `0x3f/0x55` | mode<<4\|effect, rgb, times, t1 t2 u16 | absent (listed, never built) |
| F10 | remaining system-LED effects: flash, pulse, scrolling, per-component masks | `0x3f/0x32` (SDK says `0x33`) | comp_mask, led_mask, effect, rgb, loop, t1 t2 | only solid/breath/off proven |
| F11 | hit sensitivity per plate | `armor.set_hit_sensitivity` `0x3f/0x07` | armor_mask + 9× u16 | absent |
| F12 | chassis PWM ports (accessories) | `chassis.set_pwm_value/freq` `0x3f/0x3c`, `0x2b` | mask + 6× u16 | absent; whether the S1 exposes the ports is for the manual |
| F13 | photo, digital zoom, white balance | `0x02/0x01`, `0x02/0x34`, `0x02/0x2c` — to the `dji_camera` process (host `0x01`), not the bus | type; zoom; temp/tint | ours is V4L2 (stage 7): a photo is a grabbed frame, zoom a crop; no bus command exists |
| F14 | DDS subjects not asked for: gimbal position, chassis mode, move mode, sticks | uids `GIMBAL_POS`, `GIMBAL_BASE`, `CHASSIS_MODE`, `MOVE_MODE`, `STICK` | 8 bytes each in the request | 7 subjects asked; the gimbal's real angle is what F1/F3 need |
| F15 | chassis work mode (sent at arm: mode 1) | `0x3f/0x19` | 1 byte | done; other values unproven |

Already assigned elsewhere by the owner (2026-10-29), kept: blaster fire `0x3f/0x51` (type<<4\|times;
IR by default, gel only with the key and the tank removed) → phase 3; the stock's video stream control
`0x3f/0xd2` → phase 3 (`mediad` replaces it); DJI's on-board detection `0x0a/0xa3…0xab` (person,
gesture, line, marker, robot; marker colour) goes to the `dji_vision` process, not the motion
controller → phase 2, where reusing `dji_vision` is an option against training our own; serial number
and versions `0x00/0x51` → phase 4.

Not on an S1 (EP only): servo `0x33/0x15–0x17`, gripper `0x33/0x11`, arm `0x33/0x13–0x14` and
`0x3f/0xb5–0xb8`, sensor adaptor `0x3f/0xf0`, chassis UART `0x3f/0xc0–0xc1`, ToF and pinboard (DDS),
UWB `0xdb`, AI module `0xe9/0xea`, the SDK gateway `0x3f/0xd1/0xd4/0xd5`.

What the stage-1 capture must still catch, repeated after stage 10's reboot — the owner presses, one
at a time with a 3 s pause: the follow toggle (F1, F7); recentre (F2); drag the gimbal slowly (F3, F4);
each sound (F8); zoom and photo (F13); change the LED effect (F10); hit the armour at high and low
sensitivity (F11); power off by the button with the strace alive (F5). Each becomes a real fixture and
a byte-for-byte test.

## The S1's own list (2026-10-30)

The owner's ruling: the S1 does not extend the duck's contract, it has **its own list**, served by
`s1d` on the same socket beside the duck routes a wheeled body honours (`robot.move`, `robot.head`,
`robot.flashlight`, `robot.sound`, `robot.state`, `robot.health`, `robot.enable`). `duck-ipc-proto`
is untouched; the list lives in `s1d/src/api.rs` with its own `S1_API_VERSION`, `s1ctl` is its
shell, and every call that moves something queues the frame DJI's own library would send
(`s1d/tests/s1_api.rs` holds each to the oracle byte for byte).

| route | hardware | what it carries |
|---|---|---|
| `s1.follow` | chassis + gimbal | `mode` free / gimbal_lead / chassis_lead; `velocity_frame` body / gimbal; `gimbal_fpv` |
| `s1.gimbal` | gimbal | `recenter` (speeds), `speed` (°/s yaw, pitch), `stop`, `sleep`, `wake` |
| `s1.goto` | chassis, controller's loop | x, y (m), yaw (°), speed, yaw_speed; `stop` |
| `s1.wheels` | the four motors | rpm per wheel, under the deadman |
| `s1.lights` | 2×8 gimbal LEDs, 4 chassis plates, blaster LED | effect, colour, times, groups; `blaster` |
| `s1.blaster` | blaster | `times`; `gel` only with `--blaster-gel` and the tank out, else infrared |
| `s1.armor` | the six hit detectors | sensitivity `k` |
| `s1.pwm` | the six PWM ports | duty, hz, mask |
| `s1.volume` | speaker | percent, through the mixer control the bench names |
| `s1.telemetry` | motion controller pushes | wheels, odometry, velocities, IMU, attitude, status word, gimbal angle, sticks, battery |
| `s1.subscribe` → `s1.event` | armour, infrared receiver, button | notifications: armor_hit, ir_hit, button |
| `s1.frame` | diagnostic | one raw frame |
| `s1.version` | — | the list's version |

Still without a route, declared: `s1.power` (the battery message `dji_sys` sends — captured next
stage 1), the camera and the microphone (V4L2 and ALSA, `mediad`'s and `pet-detect`'s mechanisms,
stages 7, 8, 11), Wi-Fi (phase 4), the MA2100 vision coprocessor (phase 2).

## What the stock binaries say about the rest of the hardware (2026-10-30)

`strings` over the daemons pulled from the robot (`out/2026-10-30-poc2/stock/bin/`), read whole,
not guessed:

- **`dji_sys` owns power.** It talks to an **M0** microcontroller over a UART (reset and power
  GPIOs, its own firmware upgrade), to an **LTC CPLD** over I²C, and to an **MA2100** — a Movidius
  Myriad 2 vision processor — over SPI (reset GPIO). "Power off now…", "Power save now…", "Send
  reboot request to battery", "Request power/boot %d after %d ms", "Send ota power down message
  to 0x%x": the robot powers itself off by a message to the battery board. The frame is not in
  any library we hold; stage 1 is repeated with `strace` on `dji_sys` while the owner powers the
  robot off by its button, and that frame becomes `s1.power`.
- **`dji_hdvt_uav` reads the button** (`/dev/input/event0`: "Key_code %d key pressed / key long
  pressed", a long press triggers Wi-Fi pairing — "Wifi Pairing is triggered") and a few GPIOs
  (`hu_gpio.c`: AMT test pins, "paring_gpio"). `s1d` already reads the same device; press and long
  press are the two events `s1.event` reports.
- **Audio.** The microphone is `dji_camera`'s: it opens the capture PCM with tinyalsa and sets
  four mixer controls by numeric id (`adc_gain`, `adc_reg`, `mic_boost`, `mic_pga`) before encoding
  AAC for the app. The `/system/audio/*.apu` files are not mixer tables but profiles for DJI's audio
  algorithms (`testAPUmodeName1`, 48000 Hz, floats). So the mic route for `arecord` is the mixer
  state the stock leaves behind: the capture stage dumps every control while the stock runs
  (`amixer -c 0 contents` through the chroot) and stage 8 replays it. The speaker's volume is not a
  mixer control: `dji_hdvt_uav` sets it through a kernel device, `/dev/speaker0`
  (`duss_hal_speaker_set_volume`); the capture stage traces that ioctl while the owner changes the
  volume in the app, and `s1.volume` follows what it shows.
- **Power.** DJI's common command set has `0x00/0x45` sleep control, `0x00/0x46` shutdown
  notification, `0x00/0x47` power state and `0x00/0x0b` device reboot, and the battery its own set
  (`0x0d`). Which one `dji_sys` sends on the button is what the capture stage records (`strace` on
  `dji_sys` while the owner powers the robot off at the end).
- **The vision coprocessor (MA2100)** is where DJI's detectors run (`dji_vision`); it is hardware
  phase 2 may drive, and it is listed here so it is not forgotten.


The lab's capture parser read the command bytes at the wrong offset, so earlier notes name
`CmdSet 0x00/CmdId 0x3f` and friends; the real pairs (bytes 9 and 10 of the frame) are
`0x3f/0x60` heartbeat, `0x04/0x69`, `0x48/0x08` (telemetry push, 176 B) and `0x0a/0x53`. And the
heartbeat's trailing CRC is the value `0x51a3` — the lab note wrote its wire bytes `a3 51`.

## The ladder of risk, and the DJI app

Nothing in phase 1 writes to `/system` or to any partition: the stock services are stopped at
run time and a reboot brings everything back (measured on 2026-10-29). That is rung 0. The chroot
in `/data` with the daemons started by hand is rung 1 — still volatile. Rung 2, a persistent
autostart, is phase 4's and is only entered after stage 2's dump has been **restored once and
verified by hash**, because no public source documents a recovery path for this controller.

The app and our daemons cannot drive the body at the same time — both would write to one UART —
but they alternate by boot: a flag file in `/data` chooses, and the app stays whole because it is
also the maintenance door (its Lab is how root adb is re-enabled after every power cycle).
Firmware updates happen only in the app's mode, with root adb off.
