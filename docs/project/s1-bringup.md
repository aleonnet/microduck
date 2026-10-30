# S1 bring-up: the RoboMaster S1 as a microduck body

What a DJI RoboMaster S1 does with the microduck daemons. Everything below was measured on the
machine named in each line — the robot, or the Mac that builds for it — and nothing is inferred
from behaviour alone. Where a line says **pending**, nobody has measured it yet and the plan says who
does: the bench stages need the robot plugged in and a person at it.

The design — what of the microduck enters, what is rewritten, and where the body service sits in
place of `robotd` — is phase 2's page; this is the record phase 1 writes as it goes. The plan is
the master plan "S1 ← microduck" (plans directory, 2026-10-29) and the night diary beside it.

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
| 3 | `exec` | a binary of ours runs on the robot (`/data`, or `setenforce 0` by the owner, or the microSD); the chroot answers `uname -a` and `ldd --version` | pending — **gates the port** |
| 4 | `stop` | `hdvt_uav` and `monitor` stopped; nobody holds `ttyS3` | pending |
| 5 | `drive` | `s1d` + `netpadd` + `robotctl health` in the chroot; `duckctl udp-pad` on the Mac drives wheels and gimbal; DDS telemetry in `robotctl monitor` | pending — **gates the port** |
| 6 | `events` | `robotctl flashlight` lights the LEDs; the full `0x3f/0x33` from the app's strace reproduced; armour hits, IR and the button in `s1d`'s log | pending |
| 7 | `camera` | `v4l2-ctl` formats and one NV12 frame → PNG | pending |
| 8 | `audio` | mic as a 60 s stream through `arecord`; `aplay` while driving, CPU sampled | pending |
| 9 | `app` | what the DJI app does with the stock stopped | pending (informative) |
| 10 | `restore` | reboot brings the stock back; the app drives | pending |
| 11 | `frames` | `gst-launch-1.0 v4l2src` — `mediad`'s element — at 720p and 360p: fps and CPU | pending — **gates the port** |
| 12 | `infer` | `s1-infer` (the detector's ONNX path) on the S1 with `libonnxruntime` armv7 and on the Mac over the same frames: ms/frame on both tiers | pending on the robot; the Mac tier ran tonight on a synthetic frame (below) — **gates the port** (one tier suffices) |

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

## Two corrections to earlier notes

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
