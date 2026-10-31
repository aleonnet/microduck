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
| 5 | `drive` + `items` | `s1d` + `netpadd` + `robotctl health` in the chroot; `s1-e2e robot` from the Mac: the pad over UDP into `netpadd` (wheels, turn, strafe, gimbal, lights, sounds, arm/disarm), the S1 list through `s1ctl --tcp`, telemetry | **wheels and gimbal done 2026-10-30** (below, owner's witness); **every item pre-tested on the Mac** (`s1-e2e fake`, 40 of 40, below); the robot session runs the same program minus the two wire-only items (a test holds the two lists equal); telemetry **done 2026-10-30 (night)**: the controller's own boot-time push and the gimbal's `0x04/0x05` decoded by layout, held to two real captures and confirmed on the robot (stages `telemetry`, `telemetry-gimbal`, below) |
| 6 | `events` | `robotctl flashlight` lights the LEDs; every effect of the real `0x3f/0x32`; armour hits, IR and the button as `s1.event` | pre-tested on the Mac (events fed to the fake body reach `s1ctl events`; the button through a FIFO); pending on the robot |
| 7 | `camera` | formats listed; the frame waits for `s1-cam` (a generic stream rebooted the robot twice, 2026-10-30) | **measured 2026-10-30**, frame pending |
| 8 | `audio` | mic as a 60 s stream through `arecord`; `aplay` while driving, CPU sampled | speaker **proven** (owner, four times); mic: device `hw:0,0` right channel, no signal yet (2026-10-30) |
| 9 | `app` | what the DJI app does with the stock stopped | pending (informative) |
| 10 | `restore` | reboot brings the stock back; the app drives | measured twice 2026-10-30 (stock back, app drove), artefact pending |
| 11 | `frames` | frames at 720p and 360p: fps and CPU — by `s1-cam` over the stock's own call sequence (no `gst-launch v4l2src`: same generic path that rebooted the robot) | pending — **gates the port** (360p was never captured from the stock; a 360p `S_FMT` is a hypothesis and does not go to the robot) |
| 12 | `infer` | `s1-infer` (the detector's ONNX path) on the S1 with `libonnxruntime` armv7 and on the Mac over the same frames: ms/frame on both tiers | pending on the robot; the Mac tier ran tonight on a synthetic frame (below) — **gates the port** (one tier suffices) |
| — | `telemetry` | with the stock alive and nothing of ours on the UART: the stock controller process relaunched under `strace` from its first write (its boot-time DDS subscription, the subject list of the 176 B push) and 40 s of UART reads while the owner drives the gimbal to its stops (the `0x04/0x05` push against the manual's limits) | **done 2026-10-30, 20:55 and 21:0x** (subject list captured; gimbal push confirmed at the pitch stops, below) |

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
the same names and the same order, minus the two wire-only checks — the test
`items_are_the_same_program` in `s1-e2e.rs` holds the two lists equal — with the real `s1ctl --tcp`
for every `s1.*` call; a person at the robot is the witness there, and only Enter or `y` passes an
item). Four items differ in transport, because `robotctl` speaks only the unix socket and the Mac
reaches the robot through a TCP forward: `health`, `flashlight red, off`, `sound greet, chirp` and
`robot.state` go through `robotctl` in the fake run and through the same JSON-RPC calls from the
Mac in the robot run; `bench.sh drive` and `events` run `robotctl health`, `quack`, `flashlight` and
`monitor` in the robot's chroot beside it, so the duck's shell is exercised on the robot too:

| through | items | judged on |
|---|---|---|
| `robotctl health --json` | the loop healthy | the answer |
| the pad (UDP → `netpadd` → `robot.*`) | Start arms (five work-mode-1 frames, then the heartbeat); left stick forward (all four wheels, `[fr, −fl, −bl, br]` equal); right stick turns (sides opposed); left stick strafes; sticks centred (wheels zero); D-pad right + right stick (a gimbal frame with yaw and pitch both non-zero, read back from the frame); stick centred (the last gimbal frame is `(0, 0)`); Home twice (LEDs white solid, then off); RT and LT (one `aplay chirp.wav`, and **one** `wheee.wav` for a 300 ms hold — the hold is a level); Start disarms (no heartbeat for 300 ms) | the wire, and `aplay`'s stand-in |
| `s1ctl` and the socket | version; `robot.enable`; F16 wheels (`[100, −100, −100, 100]`, zero after `robot.stop`); **the deadman** (one `robot.move` and silence: 20–70 moving wheel frames, then zero, no stop sent); F1 follow gimbal_lead (gimbal work mode 0, stick overlay 1, the twist as `0x3f/0x22` with no wheel speeds), chassis_lead (work mode 2), F7 velocity frame gimbal (overlay 2), F4 FPV (work mode 1), free; F2 recenter; F3 speed 30/−15 then stop; F5 sleep, wake; F6 goto 1 −0.5 −90 then `--stop`; F15 chassis work mode 2 then 1 as raw frames; F10 lights solid red, breath blue, flash green, marquee white, off (effects `[1, 2, 3, 4, 0]` on the wire); F9 blaster LED; `robotctl flashlight on --color red`, off; F11 armour 1.0; F12 PWM 500 ‰ / 1 kHz; F17 blaster infrared (three shots: the fire frame, then per shot the stock's own muzzle flash — `0x3f/0x55`, control 7, effect 3, green, 40 ms, captured in `capture/hdvt_fire.strace` and held as `gun_led_fire_stock_real.bin` — and the stock's `ir_gun_shoot.wav` through `aplay`, on the launcher's 330 ms cadence), gel refused naming the key; volume 50 (`amixer -q -c 0 sset Speaker 50%`); `robot.sound greet` and `robotctl quack` (`aplay greet.wav`, `chirp.wav`); a raw frame | the wire: the oracle hex of each command (`dji_oracle.rs`); where a builder is used instead, that builder is itself held to the oracle or to a real fixture |
| telemetry | the subscription on the wire (add-node for the script node `0xC9` in the stock's layout — `dds_add_node_stock_real.bin` is node 9 — never a node reset, nine subjects under msg `0x5d` with flags 3); a push of ours fed to the body → `s1ctl telemetry` reads battery 77 %, gimbal `[90, −15]`, picked up, x 1.5 m, wheel 123 rpm, the controller's answer `0x00` logged as accepted, and the push **acked with the stock's exact frame, the push's sequence included** (`dds_push_ack_stock_real.bin`; `fake_body.rs` holds the same for the stock's push and ours); `robotctl monitor --json` → `robot.state` head yaw 1.5708 rad, odometry x 1.5, picked up | `s1ctl`, `robotctl`, the wire |
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
   one start per hold, the release stops it, and a one-shot arriving mid-ride is dropped as
   `robotd::sound::play` drops it (the second round found the chirp cutting the ride and the next
   tick restarting it); held by a unit test with a stand-in player — which waits for the stand-in's
   line with a deadline, after a fixed 200 ms sleep failed 2 runs in 35 under load in the second
   round — and by the pad item, which counts exactly one `wheee.wav` for a 300 ms hold.

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

One choice the pre-test could not settle, settled on the robot (2026-10-30, night): `s1.blaster`
addresses the blaster host (`0x17`) directly, while DJI's library sends the fire command to the stock
controller (`0x09`), which is stopped under us. The launcher fires either way (six infrared-hit
events per three shots, reflected by a white wall, `items/F17_infrared_fire3.log`); what the stock
controller adds beside the fire — the muzzle flash and the shot sound — is the controller's own
doing (`gunShootAction`, `ir_gun_shoot` in its strings), captured on the UART with the stock alive
and the app firing (`capture/hdvt_fire.strace`: 42 fire frames `0x3f/0x51` from `0x09` to `0x17`,
no ack; 30 muzzle flashes `0x3f/0x55` control 7, effect 3, green, once, 40 ms; nothing from the
launcher back on the UART but its version string), and reproduced by `s1d` per shot. The owner saw
the green flash and heard the shot with our daemon (`F17_refute.txt` is the refutation).

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

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/e2e_fake.txt sha256=fcc5df8a73ea32ccf839e8474a55ee29eb2988ff66bd8fa7e66b1e3aaaf003a7
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/work/report.txt sha256=1db1dbfba84c9dadc6c5bc26dc044ce26b19f0476777a166999c471bf84f4ed5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/e2e/work/wire.bin sha256=e5f1897546550f1c36b64425a82c00821081562b6c8c919a09fcc73d3a37ce75
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
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/cmd.txt sha256=de63dc28afbc5b69091ec0882896bf5318af74d7bca99e6587ecaeb91c63c461
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/df.txt sha256=b9e3fc31b26f413c6d8bea8aeabde187fe27483437370865098b64eff7313ae1
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/dji_camera.strace sha256=6fef0cdba8efa52ee792cfe25e1c6c89950a4951e02036ddb3553868820d7713
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/getenforce.txt sha256=1535895fea3509918515479905fb814e80657ea5db44bcd7bbdf6ffbde370de2
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/getprop.txt sha256=6e8e4fffb2b439e910e83a22c336d1d819daefff3e872beaa083c362720e5a8e
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/hdvt_uav.strace sha256=bb21da55392e3e2c4a80d191a2e8b410c1988bbabb45804e6eed043d84e89054
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/ls_-l_dev_block_platform_by-name.txt sha256=c7fbf3de2a68018357f375d2478d0ffe38dd26ef1af067f2f63494159ec7b04e
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/ls_-l_dev_input_dev_video_dev_snd.txt sha256=d01c90f276822fd84c39c9bb2f720bb701b7d3023edc52959a29f1df710e484d
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/ps.txt sha256=151fa1ba757ec9e7364a6cca2d8bfc45bfc879451601300efeeb959873e3c317
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/top_before.txt sha256=264f0f426ea57d1b4599622e960997b37035f410cdd0bbe2b0b3c9d1ab634e8e
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/top_vision.txt sha256=8de938b387193822a9c1719158d06d9ebd890f10b333385122eabe25c5bc0512

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
   Superseded 2026-10-30 (night): the stock's push is now decoded by its own layout (section "Telemetry, read off
   the pushes that already flow"); it is the telemetry the robot actually gives.
2. The controller then ignored our subscription outright. Running the stock `dji_hdvt_uav` under
   `strace` for 10 s (`hdvt_boot.strace`) shows what it sends on start: `0x48/0x01` register node 9
   (`09 00 00 00 03`), then `0x48/0x03` subscribe — both with attribute `0x40`, the SDK's
   `need_ack = 2` ("ack when finished"), a two-bit field our frame modelled as one bit. Fixed:
   `Ack::{None, Now, WhenDone}`, `dds::add_node`, both requests `WhenDone`; the two stock frames
   are fixtures reproduced byte for byte. The controller now **answers** both requests.
3. Open: the answers are refusals — add-node returns `02 03` (the SDK accepts only `0x00` and
   `0x50`), subscribe returns `21 03 00 5d 00…`. A `0x48/0x02` node reset was tried (night): the
   controller then answered `0x0f` to everything, and the stock's own `dji_hdvt_uav`, restarted
   under strace, got no answer and no push either — **a reset leaves the bus dead until a power
   cycle**, so `s1d` never sends one (`fake_body.rs`, F18 item). Under the script runtime's node
   `0xC9` on a fresh boot (reboot, 2026-10-30 15:54) the answers are the same `02 03` / `21 03`
   as under node 9. Next lead: how DJI's runtime subscribes (`rm_module.py`, msg ids and the
   `0x48` requests it sends through the stock controller).

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/exec/exec.txt sha256=3d38161894241b029aa2db218b42271da30344015b1e236c1e0f6cf1364d96a0
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/stop/stop.txt sha256=93993a72aca42aaa5d68f7f5908a965278cb4465616b8ad9b55f1e7cef566425
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/stop/owners.txt sha256=dfa3ab767dc85cd9b759818b95b1d7df1a7ec15dd211b83eb3ca97481c099705
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/health.json sha256=d3817efd88f12c66e52e1c8c20983a4e669aa284cf93009533616be7edb9dfe5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/move.log sha256=3d3ef2f5bb7664844e21a33e67e6c1b789ef03bbceb0caf06aa49d65e16f1f6a
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/drive.py sha256=543331d05db8d47ae0e3fdbf9235bcfe9d8a1e1f2a6634ec9fdbef36886a52da
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/hdvt_boot.strace sha256=41ca4f9d1f03599dc419096895f1df7a2248240b82a49114c44bb6eb62b34ddb
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/monitor_before_fix.log sha256=324ad59be5e3e6f11ef6036dd585230b2a5d077e77b0fcf5c53d475bcf263432
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/s1d_trace_after_ack_fix.log sha256=390cfee9e4fa22963ed40ec86e75f0b533a13c37cb6bbab98c280c65543c86fa
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/backup/mirror-check-20261030T122913Z.txt sha256=b84188f7ca61a2eaaf4db2e1776d8a261d3c9caa69eabb411e032b9fd31e1797

## Robot session, 2026-10-30 (night) — stages 5 and 6 on the duck's mechanism, the owner watching

Every item below was run by `s1-e2e robot` or by `s1ctl`/`robotctl` over the socket (`adb forward
tcp:9000`), the pad over UDP to the robot's `netpadd`, and judged by the owner in the chat before
the next one ("A cada teste que fizer me diga aqui para te dizer se VI a execução de fato").
Wheels off the ground throughout; the launcher aimed at a white wall.

**Seen by the owner:** chirp by `robotctl quack`; the pad end to end (Start arms, forward, turn,
strafe, release, gimbal by D-pad right, Home white on/off, RT chirp with LT wheee held without a
stutter, Start disarms); `s1ctl version`; F16 wheels 100 rpm; the deadman (one move, stops alone);
F10 five body-LED effects; F9 the launcher's trail light; F11 armour hits on every plate and the
controller button as `s1.event`s (23 hits, plates 1–4, and two button presses, codes 264 and 265,
in one 60 s window); F17 three infrared shots with the muzzle flash and the shot sound. The gimbal
moved left/down for "right/up" on the first try: DJI's signs are yaw positive = right, pitch
positive = up (`rm_ctrl.py rotate_with_degree`), the duck's the opposite; `tick::gimbal_ddeg` and
`head_from_gimbal` now convert, and the owner saw "direita e cima".

**Accepted, not observable in the air:** follow modes F1/F4/F7 (the body must be on the floor to
turn after the gimbal), F6 goto, F11 armour sensitivity, F12 PWM, F15 chassis work mode. They stay
🟡 until a floor session; F1 is the owner's priority ("é um movimento de destaque quando tivermos a
inteligência").

**Telemetry (F18), still refused.** A `0x48/0x02` node reset was tried once: afterwards the
controller answered `0x0f` to everything, and the stock `dji_hdvt_uav`, restarted under strace, got
no answer and no push either — a reset leaves the bus dead until a power cycle. `s1d` never sends
one now, and registers DJI's script node `0xC9` instead of the stock's 9; on a fresh boot (reboot,
15:54) the answers are the same `02 03` to add-node and `21 03` to subscribe as under node 9. Next
lead: the runtime's own subscription path in `rm_module.py` (node `0xC9`, msg id 0, through the
stock controller), and the stock's live DDS traffic in `capture/hdvt_fire.strace`.

**The muzzle flash, and two errors of method.** `s1.blaster` fires (six infrared-hit events per
three shots, reflected by the wall, 330 ms apart) but showed no light and no sound: DJI's runtime
sends the fire command to the stock controller, which adds both itself. The sound is the stock's
`ir_gun_shoot.wav` through the speaker (now `aplay` of the same file, once per shot). The light I
first sent by hypothesis — the trail-light command in solid green, then a body-LED frame with
component bit 6 — and neither lit: three frames went to the robot without a captured or
library-built source, which is the trial-and-error the owner forbids. The capture that settled it
(reboot to the stock, strace of `dji_hdvt_uav` and `dji_sys` while the owner fired in the app)
shows the flash as `0x3f/0x55` with control 7, effect 3, green, once, 40 ms
(`gun_led_fire_stock_real.bin`); with that frame the owner saw the green and heard the shot. The
same capture was the moment to collect every app behaviour still unseen (follow modes, FPV, gimbal
sleep, goto, power-off) and I asked only for shots: the next capture follows a written script. The
"no SD card" the app showed after the reboot is unexplained; the card is mounted and holds the
backups (`df`: 29.8 GB, 3.6 GB used).

**Stage 10 (restore) measured, artefact pending:** after `reboot` the stock came back by itself
(`pidof dji_hdvt_uav dji_sys` → 233 243 at 15:43, the app connected and fired); `restore/after.txt`
was not written because the daemons were stopped again for our own session, so the stage stays 🟡
until a run that ends with the stock alive.

**Bench defect found:** a reboot drops the chroot's `/dev`, `/proc`, `/sys` and `/mnt/system`
mounts of stage 3, and `s1d` dies at start ("failed to generate random data": no `/dev/urandom`).
The daemons were started by hand after re-mounting; the stage that starts them must re-run the
guarded mount line (to fix, with the self-test).

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/e2e.log sha256=0b3cf9f587c0920444d46848305df24533d7c81ebb6e3f068c73c64a7e1585f5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/items/e2e.log sha256=11233b0a47286b123693d64642bce72db3606c0a1037dfb026100bca8f5cf954
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/drive/hdvt_fresh_after_our_reset.strace sha256=7ebc4458b6a1b066c95bb1c1b36e7ef5c78ba778276c9ad9fabd1e490a3eb3b8
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/events/e2e_window3.txt sha256=180758239c26b88aaf623606b3d346048e27238ab181b9f7bd97ca540517b95e
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/events/s1d_window3.log sha256=36a3703ccc5f9240c86eca5dc818403d41d0b347d8f0dd88b1a058671fa8a379
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/items/F17_infrared_fire3.log sha256=6c895b62fe0ca03d973b3ba900514167d3dd0d00bc30e6850262e0d3c2b85f94
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/items/F17_refute.txt sha256=997eae479b0a34dd82230d33fa1896d45ab8e322a3bbfc3caffdd6388fca99d5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/hdvt_fire.strace sha256=fb5224197975a4dc05b21305cd7c1d5beb328f20178ab0a5c9b2151465c1e3ba
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-20261030-165821/sys_fire.strace sha256=36ba6d9f66a2574b3339a983496411b848b2ec454e589c2e0d2f2654bc0a7fda

**Capture re-run of 16:58 (2026-10-30), and what it cost.** The first capture of the next battery
ran without the app: its output was piped through `tail`, so the line that should have told the
owner "window open" never reached the chat, and the 150 s passed with the robot idle (two body-LED
frames, no fire, no follow). Its files **overwrote the morning's stage 1 artefacts** of the same
names (`cmd.txt`, `df.txt`, `dji_camera.strace`, `getprop.txt`, `hdvt_uav.strace`, `ps.txt`,
`top_before.txt`, `top_vision.txt`): the hashes below are the 16:58 files; the morning's strace,
the one the LED fixtures came from, is lost as a file (the fixtures and the decoded tables survive
in the record above). The bench now moves an earlier run aside under its stamp before a stage
re-runs (`dir()`, proved in the self-test). The same run restarted `dji_scratch` at 60 s to put its
telemetry subscription on the UART: `stop dji_scratch` also SIGKILLed `dji_sys` (the power daemon,
pid 246, 16:59:24; init brought it back as 27689) and no `0x48` request appeared on the UART — the
restart is out of the script until `dji_scratch.py` and the service group are read.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/hdvt_uav.strace sha256=b342068538f8ec0ed66a7bc218e665f3e1d3bcf3f0a9fe0fae60384302816eb5
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/dji_camera.strace sha256=ac9a6c8e41b1bce3ce129a3ffde755b1ac796304cf1ba917e549dc92f0a7cf69
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/dji_sys.strace sha256=1a5e6b8e9d87e47ed11c73c0497cf7f496064955221c84f7502b722663c280b0
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/cmd.txt sha256=086041d64a61c78d9e0ecb6560b55a0059a4664cd99c8397cbf561b5b7757f23
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture/top_vision.txt sha256=148097cba43d81d292946d6e10945dec5ec524042ed9e4ad5443ebf887a035df
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/capture-follow/hdvt_follow.strace sha256=5c050a96cc9ceced0cd4f5a4e6ed29b7073d2d9ab3175265ac010e2d9a41d315
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/camera/formats.txt sha256=f8f22089c7f60bf0e726fba0f56568751f4113761b0d90c24511a149128aea2b

**Captures of 17:05 and 17:12 (2026-10-30), the owner on the app.** The full script ran in the
17:05 window (`capture/`, three straces): 101 fire frames, 38 body-LED frames, 12 muzzle flashes,
gimbal work modes, and at 17:07:22 a sequence from the script runtime (sender `0xC9`: wheels zero,
gimbal angle and recentre, then twelve pairs of gimbal speed ±120°/s with chassis yaw ∓120°/s) —
an app behaviour running as a script; the window closed before the last steps. The app was very
slow under the three full straces, so the follow steps were captured again at 17:12 with **writes
only, `dji_hdvt_uav` only** (`capture-follow/hdvt_follow.strace`, 90 s), and that is what the
record keeps for the modes: "chassis follows gimbal" → gimbal work mode **0** (`0x04/0x4c 00 00`,
twice; the chassis side had gone at app connect: `0x3f/0x19 00` to the motion controller and
`0x3f/0x66 02 00` to the chassis, sender `0x02`, 17:05:55); "gimbal follows chassis" → gimbal work
mode **2** (`0x04/0x4c 02 00`) repeated at 8 Hz for as long as the mode is on (122 in 15 s). Both
values are the library's own (`gimbal_free_mode`, `gimbal_yaw_follow_mode`). There is no separate
"FPV" toggle in the app (owner, 2026-10-30: the piloting screen *is* FPV): what the library calls
`chassis_fpv_mode` is the app's default driving, and the frames the app sends at connect are its
whole setup (`0x3f/0x19 00` to the motion controller, `0x3f/0x66 02 00` to the chassis, both from
sender `0x02`); F4 needs no further capture. The mic and the volume do not pass the UART (camera
and ALSA processes) and are stage 8's.
The power-off trace of `dji_sys` (`/data/sys_poweroff.strace`, 17:15) stays on the robot for the
next boot. Stage 7 started after the owner closed the app: formats listed (YUYV, NV12, NV21, two
10-bit Bayer), then `VIDIOC_S_FMT` 1280×720 NV12 "Invalid argument" **while the robot was going
offline** (powered off by the button at the end of the script): inconclusive, to repeat. Stages
8, 11 and 12 were not reached.

**Stage 7, the camera: two reboots and the stock's own setup, decoded (2026-10-30, evening).**
`v4l2-ctl --set-fmt-video=… --stream-mmap` on `/dev/video0` (stock camera process stopped) made the
robot drop off the network at the same point twice (17:01 and 18:3x); after the second the robot
answered with an uptime of 55 s — a reboot, with no `/proc/last_kmsg` to read. **No more streaming
by a generic tool.** The stock camera process was restarted under the chroot's `strace -v`
(`camera/dji_camera_v4l2_decoded.strace`) and its setup is now known field by field: `QUERYCAP`
(driver `comip`, capture + streaming) → `S_INPUT 0` → three private controls `S_CTRL`
(`V4L2_CTRL_CLASS_CAMERA + 0x191b = 0`, `+ 0x1934 = 30`, `+ 0x1935 = 0`) → `S_FMT` 1280×720 NV12,
field ANY, colourspace JPEG → `REQBUFS` MMAP, 5 buffers → `QUERYBUF` ×5 → `STREAMON` → `DQBUF`/`QBUF`
at the frame rate → one more private control (`+ 0x1928`). The generic tool's `S_FMT` without the
input and the controls was refused ("Invalid argument"), and its stream request is what preceded
both reboots. The stage now takes the device's reported format without `S_FMT` and writes inside
the chroot's own `/tmp` (files named to a chroot program are the chroot's: `/data/frame.nv12`
never existed on the robot — the same defect in the frames and audio stages, fixed together, with
the self-test asserting the paths); it is **not run again until a grabber of ours replays the
decoded sequence exactly** (input, the three controls, the format with its field and colourspace,
five MMAP buffers) — `s1-cam`, the next piece of code, and the base of stage 11 and of the
`mediad` source. After the stock camera process was killed twice under strace, opening the device
from Android gave `ENODEV` until the next reboot; a power cycle precedes stage 7.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/camera/dji_camera_start.strace sha256=d11b0d55d6d7e430b575f58a5a3404a9a18d0a6469654e6811ff6a93a6c2dd48
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/camera/dji_camera_v4l2_decoded.strace sha256=ba572e0b80b6826d01245d5038902493dc098638d3ad126a821a820e3c2adcef

**Stage 8, audio (2026-10-30, 18:00–18:40), three runs (the first two archived under their
stamps).** The speaker is **proven by the owner's ear, several times over the day**: the chirp by
`robotctl quack`, the chirp and the held wheee from the pad ("Sim, ouvi os dois"), the shot sound
of F17 ("Ouvi os sons"), and the stock's own wavs through our `aplay` here; the mixer's `Speaker
Volume` control (value 20) is the one for `s1.volume`. The microphone: the first run recorded nothing — the stage used `hw:0,1` and the device answered "read
error: Invalid argument"; the stock camera process opens `pcmC0D0c` (its strace from start), i.e.
capture device 0, which accepts S16_LE, 1–2 channels, 8–48 kHz (`--dump-hw-params`). A mono
recording on it is a constant (RMS = peak = −50.1 dB); a stereo probe has that constant on channel
0 and a faint varying signal on channel 1 (`I2sL Out` = ADC2, `I2sR Out` = ADC1 in the mixer). The
stage now records stereo at 48 kHz and splits the right channel. The third run, 60 s, stayed at
−75 dB RMS throughout (the owner was not at the screen and did not speak), and a loopback with the
robot's own speaker playing `engine_start.wav` while recording also reached only −73 dB RMS on the
right channel: **the capture path is not carrying the microphone** with the mixer as the stock
leaves it (identical dumps with the stock camera process stopped and running, so its six mixer
writes at start are idempotent). Open: what the stock sets when the app records audio (a mixer
dump during an app recording, 20 s, is the next measurement), and the stock's PCM parameters
(`HW_PARAMS` undecoded). The mic stays 🟡 until a recording holds a voice or the loopback holds
the speaker.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/amixer_contents.txt sha256=38bf6a11557fc8d21a1e8a829bdd7caab5ab92e37d79c4cea8f59854795c9573
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/aplay_stock_sounds.txt sha256=88d664f490c8a6029da5f577330f0f5cc25c3f291c57c9f8479fa27df2368624
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/mic.wav sha256=641906028e37b50c8eff86f1daa58ad315cda3fd9e63c5f4cd6a1972d393e738
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/mic_right.wav sha256=64b764c6a1c2e07c878feadb1dec1b0ff81cb69d547df19c032d6f46cdfc3722
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/stream_60s.txt sha256=4025356b8d20c4efb725bad03ed2bb06ab57cfe02e72a2d4c40c9306adc0193e
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/cpu_play_drive.txt sha256=6740f5bfaea3b82e7f952eafb7588175627ac16366571a34d624227a9e302ce6
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio/loopback_speaker_to_mic.wav sha256=64422265225cc15889de549e8a5cc5de2db5fdf80abc043bd6b3490bc980dc46
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio-20261030-180316/probe_48k_stereo.wav sha256=dfc182c67be71deb2645137678bb8b2202c5bb1c7a3d707a17138c1ac6311383
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio-20261030-180316/mixer_camera_stopped.txt sha256=696f8ebcd397693a532677ff2bdfa73f2df2c553c93e866ed6dc6ea2a9351a31
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/audio-20261030-180316/mixer_camera_running.txt sha256=696f8ebcd397693a532677ff2bdfa73f2df2c553c93e866ed6dc6ea2a9351a31

## Telemetry, read off the pushes that already flow (2026-10-30 (night))

**What the raw captures say (CRC-checked frames, decoded independently of the Rust code):**
with the stock alive (`capture/hdvt_uav.strace`, 17:05–17:07, the UART's reads) the motion
controller (`0x03` → `0x09`) pushes two `0x48/0x08` subscriptions it was given at boot: one of
176 B at 50 Hz nominal (median gap 20 ms; 2362 in the window) and one of 69 B at 1 Hz (59). Their
`msg_id` swaps between boots (0/1 at 17:05, 1/0 in the daemon's own run at 14:30), so the push is
recognised by its length. With `dji_hdvt_uav` stopped the pushes go on: the daemon's trace log of
the 14:30 run counted 399 of the big one and 8 of the small one, all discarded as "foreign".

**Layout of the 161-byte payload** (after `sub_mode`, `msg_id`), fitted on all 2362 pushes and
holding on the 2026-10-29 idle push too — `dds::StockPush`, `s1-duml/tests/stock_push.rs`:
`0..8` time (ms, then µs) · `8..22` one subject the SDK does not name: a float that tracks the
gimbal yaw in whole degrees (0, −1, −178 against the gimbal's own −179.6°) and ten zero bytes ·
`22..46` velocity (world xyz, body xyz) · `46..82` ESC (rpm, angle, timer, state ×4) · `82..92`
battery (mV, 0.1 °C, mA, %, recv) · `92..104` position · `104..128` IMU (acc in g, gyro in rad/s) ·
`128..132` status word · `132..144` attitude (yaw, pitch, roll, degrees) · `144..161` 17 zero bytes
(the servo subject). Physical agreement is the proof: 11.94 V / 31.6 °C / −1060 mA / 80 % at 17:05
and 11.862 V / 33.2 °C / −1129 mA / 77 % on 2026-10-29; acc_z = −1.00 g on both; on 2026-10-29
the pitch is −6.6° **and** the controller's own "on slope" bit is set in the same push; at 17:05
the yaw goes from −0.7° to 19.3° as the owner turned the robot, gyro_z reaches 1.65 rad/s, body
speed 0.44 m/s, the "slip" bit shows twice. The order is the stock's own request, which was never
captured: it is sent in the first milliseconds of `dji_hdvt_uav`, before a `strace -p` can attach
(`hdvt_boot.strace` holds only the 1 Hz subscription, msg 0, one subject `00020003_d7f5dcfb`).

**The gimbal angle is not in that push.** It travels on its own frame, `0x04/0x05` from the gimbal
(`0x04`) to host `0xf1`, 40 B, 2969 in the window (~21 Hz) — `events::GimbalPush`: pitch at 0 and
yaw at 4 in tenths of a degree, a second yaw at 8, the gimbal's clock at 12, a quaternion at 24.
Pinned by the extremes: pitch reaches +35.8° where the manual's limit is +35°, yaw ±179.6°, and
the quaternion agrees with the yaw (−179.6° gives w ≈ 0.006, z ≈ −0.95). The yaw at 8 is 21° less
than the one at 4 when the chassis had turned 19°: ground frame at 4, relative to the chassis at 8
— consistent with the data, confirmed by stage `telemetry` driving the gimbal to its stops.

**What changed in `s1d`:** a `0x48/0x08` of 161 bytes is decoded as the stock's under any msg id
(`tick::Push::Stock`), ours stays by id and exact length, anything else is counted as foreign and
left alone; `0x04/0x05` fills the gimbal angle; `robot.health` carries the controller's battery
(volts, percent); `robot.state` takes odometry, attitude, the status bits and the gimbal's real
angle in `head`; `s1.telemetry` adds `stock_pushes`, `gimbal_yaw_vs_chassis_deg`, `gimbal_quat`.
Our own subscription is still sent once at start and re-asked only while nothing flows — the
controller refuses it (`02 03` to add-node, `0f …` to subscribe, also `21 03` on another boot;
cause undetermined) and it is no longer needed for the telemetry the duck consumes. Fixtures cut
from the capture: `dds_push_stock_{idle,moving,turned,1hz}_2026-10-30.bin`,
`gimbal_push_{first,yaw_max,pitch_max}_2026-10-30.bin`. Refutation: `scripts/s1/telemetry-refute.sh`
plants the battery ten bytes early and the layout tests fail naming it (`left: 11`, `right: 11940`),
the source restored by hash.

**What the duck's contract receives from it:** `RobotState.odom` ← position + attitude yaw;
`imu.gyro` ← gyro; `safety.fallen` / `picked_up` ← status bits 9 / 4; `HealthResult.battery` ←
mV/1000 and %; `head` ← `0x04/0x05` converted to the duck's signs. Only the S1 (`s1.telemetry`):
wheel rpm and angles, velocities, the whole status word, temperature and current, the quaternion.

**Confirmed on the robot (2026-10-30, 20:50–20:55, stage `telemetry`, nothing of ours on the
UART):** `dji_hdvt_uav` relaunched under `strace -e trace=write` from its first instruction sent,
in its first seconds, three `0x48/0x01` add-node for node 9 and three `0x48/0x03`: msg 0, flags 3,
one subject `0x00020003_d7f5dcfb` at 1 Hz; msg 0 again with two, `…d7f5dcfb` and
`0x00020003_91a13d9c`, at 1 Hz; and **msg 1, flags 3, two subjects `0x00020003_47e6f7b3` and
`0x00020003_1d13ee42`, frequency field 20** — this is the 176 B push: two composite subjects of
DJI's internal namespace `0x20003` (not the SDK's `0x20009` singles), together 153 bytes after the
time stamp. The controller's own clock in the pushes puts them **20 ms apart** (1314 of 2362
consecutive gaps in the 17:05 capture; the rest multiples of 20 ms, i.e. drops), so the field's
20 is not hertz on this firmware; the 1 Hz push's gaps are 1000 ms. The 14 bytes at 8..22 are
therefore part of the first composite subject, whose internal layout DJI does not publish; the
float there tracks the gimbal yaw (0, −1, −178 against the gimbal's −179.6°). The `msg_id` of the
big push was 0 at 17:05 and 1 at 20:50: by length, as coded. The stage's first run hung: busybox
`timeout -t 45` sent `strace` a TERM and it kept tracing for minutes; the owner killed the relaunched
process by hand and the stage then restored init's service (`hdvt_uav_restored`); the stage now
ends the tracee itself.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/telemetry/relaunch.txt sha256=b2d36ce6ad5f548da690ff399a7c31a5598c88c64609e3c668a41f4d9d96c690
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/telemetry/hdvt_exec_writes.strace sha256=adae9b91c641ee3637693b6a975f33fb13e403c3fc9ce199b551ca4f80fc9e41
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/telemetry/dds_requests.txt sha256=779f4156981597a184490772363accd7aac97658c06bd8988d9e813d5a935842

**Confirmed on the robot (2026-10-30, 21:0x, stage `telemetry-gimbal`, app connected, the owner
driving the gimbal to its stops for 40 s):** 1388 `0x04/0x05` pushes in 35.8 s (38.7 Hz in this
window; 21 Hz in the 17:05 capture — the rate varies). Pitch field −21.2°..+36.8° against the
manual's −20°/+35° (the mechanical stops sit a degree or two past the published figures), first
excursion of yaw negative when the owner was asked to go left first (DJI's sign: right positive),
yaw −95.1°..+121.4° (the yaw stops at ±250° were not reached in the window). The quaternion agrees
with the yaw field to within 1.2° at both yaw extremes (−94.7° for −95.1°, 122.6° for 121.4°). The
yaw at 8 stayed near 0° while the yaw at 4 held 92°–100°: the chassis was following the gimbal, so
8 is the gimbal relative to the chassis and 4 is the ground frame — as read from the 17:05 data.
`events::GimbalPush` is confirmed field by field; telemetry is closed.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/telemetry-gimbal/hdvt_gimbal_reads.strace sha256=cefd0ab0a759b8339ac1e4788f2989e6638cf172cd884b8a753cd6db1e3d45e7
- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/telemetry-gimbal/gimbal_pushes.txt sha256=a0ceece4b79f30366f6d13f500470e23789feb42cead8900fa0837a792d2aea4 The DJI on-board runtime (`dji_scratch`) subscribes only when a script starts
(`script_framework.py:94`): node `0xC9`, msg 0, flags 0, ten subjects at 50 Hz, accepting retcode
`0` or `0x23` (`rm_module.py:1300`, `rm_ctrl.py:5184`).

## Corrections after the cold reader (2026-10-30 18:29 -0300) — this section supersedes the hand-off's open-items table

A cold reader resumed from `docs/plans/2026-10-30-1905-handoff-s1-fase1-bateria2.md` alone and found
the defects below. **That file's name and its "19:05" are wrong: it was written at 18:15 (commit
5ad758c, `git log -1 --format=%ci`), and the stage 8 times "18:00–18:40" are 18:05–18:08 by the
artefacts' mtimes.** The scope lock keeps this session from renaming it; the next session reads it
for the list of errors and this section for the facts.

**Do not believe what this session reports about code or progress. Read the code and the disk.**
Every claim below carries its file and line; a claim without them is a hypothesis.

1. **The camera sequence, complete this time** (`camera/dji_camera_v4l2_decoded.strace`, the
   chroot's strace attached to a freshly started `dji_camera`; provenance command, run 2026-10-30:
   `stop dji_camera; start dji_camera; chroot /data/debian /usr/bin/timeout 20 /usr/bin/strace -f -tt -v -s 64 -e trace=ioctl,openat,open -o /tmp/cam_v.strace -p <pid>`):
   `open("/dev/video0", O_RDWR|O_NONBLOCK|O_LARGEFILE)` l.312 → `QUERYCAP` driver `comip`, capture +
   streaming l.313 → `ENUMINPUT 0` = `ov5695` l.314 → `ENUMINPUT 1` EINVAL l.315 → `S_INPUT [0]` l.316 →
   `CROPCAP` bounds 2592×1944 l.317 → `G_INPUT` → **[1]** l.318 → `S_CTRL` `V4L2_CTRL_CLASS_CAMERA`
   (`0x009a0000`, UAPI `videodev2.h`) `+0x191b = 0`, `+0x1934 = 30`, `+0x1935 = 0` l.495–497 → `S_FMT`
   1280×720 NV12, field ANY, colourspace JPEG, and the driver answers **field INTERLACED, bytesperline
   1920, sizeimage 1382432** l.498 → `REQBUFS` MMAP count 5 l.499 → `QUERYBUF_TIME32` ×5 l.500–504
   (pointers only: flags/field/memory NOT on disk) → `mmap2` ×5 of 1384448 bytes, `MAP_SHARED`, page
   offsets 0, 0x152, 0x2a4, 0x3f6, 0x548 (`camera/dji_camera_start.strace` l.1056–1064) → `QBUF` ×5
   l.505–509 → `STREAMON` l.510 → `DQBUF`/`QBUF` loop from l.996 (525 `DQBUF` in 17.34 s ≈ 30 fps; no
   `EAGAIN`; `poll`/`select` were not traced, so whether the stock waits before `DQBUF` is unknown) →
   `S_CTRL +0x1928` as **four values** (1073755392, 1073755393, 115056, 115057) repeated at 1 Hz on
   another thread l.1746–1752. The driver exposes no named controls (`camera/v4l2_all.txt`), so
   `v4l2-ctl --set-ctrl` cannot replay them; `s1-cam` must issue the ioctls with the ARM32 UAPI
   layouts (`v4l2_buffer` with 32-bit timeval), which neither the repo nor the rootfs carries
   (`mediad/src/exposure.rs:381` chose `v4l2-ctl` to avoid pinning those layouts by hand).
2. **The bench no longer streams with a generic tool.** Stages 7 and 11 stop before the stream and
   record `blocked.txt`; `bench-selftest.sh` asserts that neither `--stream-mmap` nor `gst-launch
   v4l2src` is sent. Until `s1-cam` exists, no frame and no fps.
3. **`hw:0,1` is not "untested"**: `arecord -D hw:0,1 -f S16_LE -r 16000 -c 1` answered "read error:
   Invalid argument" (first run of stage 8); `--dump-hw-params` says it accepts S16_LE, 1–2 channels,
   8–16 kHz. It was never recorded from. The stock's capture device is `pcmC0D0c` (= `hw:0,0`).
4. **Hours**: every "19:05"/"18:40" in the hand-off and the memory for the evening blocks is about
   50 min late; the commit times are the truth (`git log --format=%ci`).
5. **Memory and index**: the live memory still holds revoked orders ("tentar reset de nó", "tentar nó
   0xC9", "dono testemunha cada item") and old counts (40/71 OK) in earlier blocks; the later blocks
   revoke them. The index line for the master plan says the front is closed; it is open (base
   `4d9ca3c`). The next session trusts the latest block and the disk, not the earlier blocks.
6. **Wheels in the air is a safety question, not a repeat**: before any stage that moves the wheels
   the owner is asked once whether they are off the ground.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/camera/v4l2_all.txt sha256=e6f155d453c71e2f124ce41494d2d89a3df78525d950548cfdd19308e4529334

## Before the next battery (written 2026-10-30, night, robot off)

The owner's rule after this session: **no frame goes to the robot without a captured frame or the
library's own bytes behind it, and a capture window with the stock alive follows the whole script
below, never one item.** What each remaining item has, column by column:

| Item | DJI's code (read) | Captured on the UART | Ours, byte for byte | Goes to the robot? |
|---|---|---|---|---|
| F1 follow, chassis after gimbal | `RobotCtrl.set_mode` → chassis work mode + `0x3f/0x22` (`Chassis.set_follow_speed`), gimbal work mode | not yet | oracle-held (`dji_oracle.rs`: workmode, follow, stick overlay) | after the capture (script item 7), robot on the floor |
| F1 follow, gimbal after chassis; F4 FPV; F7 velocity frame | gimbal work mode 2 / 1, stick overlay 2 | not yet | oracle-held | after the capture (items 8, 9) |
| F5 gimbal sleep / wake | `Gimbal.suspend/resume` (`0x2ab5` / `0x7ef2`) | not an app action | oracle-held | yes: the library's bytes are the source |
| F6 goto | `Chassis.set_position` task | not an app action | oracle-held | yes, on the floor with 2 m free |
| F12 PWM | `Chassis.set_pwm_value/freq` | not an app action | oracle-held | yes, with a load on a port |
| F18 telemetry | `Tank.set_sub_node` + `add_gimbal_and_chassis_sub_msg`: node `0xC9`, msg 0, `sub_config` 0, `mode` 0, ten subjects with high word `0x20009`, 50 Hz — sent to the **stock controller** (`0x09`), which forwards; accepts `0x00` or `0x23` | the stock's own: node 9, msg 0, flags 3, one subject `d7f5dcfb/00020003`, 1 Hz (`hdvt_boot.strace`); the forwarded script subscription was never captured (it happens at boot) | subjects as the library's, flags 3 as the stock's — refused `21 03` | nothing of ours: the stock's push is decoded as it flows (2026-10-30 (night)); stage `telemetry` relaunches `dji_hdvt_uav` under strace for its subject list and reads the gimbal's `0x04/0x05` at the stops |
| volume | — | — | `amixer sset <control>` | after stage 8 names the control (`amixer_contents.txt`; stage 1 wrote none) |

Stages 7, 8, 11, 12, read against what exists: the rootfs tar carries `v4l2-ctl`, `gst-launch-1.0`,
`gst-inspect-1.0`, `arecord`, `aplay`, `amixer`, `strace` and the GStreamer plugins
`video4linux2`, `png`, `jpeg`, `videoconvertscale`, `videorate`, `alsa`, `app`, `coreelements` —
**not** `fpsdisplaysink` (gst-plugins-bad, never installed), which stage 11 used: the stage now
reads the rate off gst-launch's own "Execution ended after" over 300 buffers, and the self-test
checks the arithmetic (24.0 fps for 12.5 s). `libonnxruntime.so.1.28.0` armv7 is built
(`out/onnxruntime/`) and `s1-infer` cross-compiles; stage 12 can run both tiers.

The bench defect of the night is fixed: every stage that enters the chroot (`drive`, `camera`,
`audio`, `frames`, `infer`) re-makes the chroot's `/dev`, `/proc`, `/sys` and `/mnt/system` mounts
first, guarded, the same line stage 3 uses (`need_chroot`); the self-test asserts it per stage (its
assertions now read only what the stage under test sent) and the refutation below removes it and
breaks the frame-rate arithmetic, each caught.

The capture script (stage 1's prompts, 150 s, three straces): 1 camera on · 2 a sound · 3 each
armour plate · 4 LED colour, then each effect · 5 the controller button · 6 fire three times ·
7 chassis follows gimbal (robot on the floor) · 8 gimbal follows chassis · 9 FPV · 10 recentre ·
11 drag the gimbal · 12 speaker volume · 13 speak into the mic · 14 person recognition · 15 (the
stage itself, at 60 s) stop and start `dji_scratch` · 16 power off by the button at the end.

- artefato: /Users/alessandro/Development/RoboMaster-S1/out/2026-10-30-poc2/items/bench_mounts_refute.txt sha256=4ed117577ea03b61fc8886886e49336258aa9c8610e8e40ba8ee66e6784f24e7

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
| F9 | blaster LED (trail light; the muzzle flash is this command with control 7, effect 3) | `blaster.set_led` `0x3f/0x55` | ctrl<<4\|effect (ctrl 7, effect as `rm_define`: on 0, off 1, breath 2, flash 3, marquee 4), rgb, times, t1 t2 u16 | absent (listed, never built) |
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
