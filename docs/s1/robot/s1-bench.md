# Running the S1 from a Mac

How to take a RoboMaster S1 from cold to running our software, by `adb`, and what each step has really
proved. What the steps found is in [`docs/s1/project/s1-bringup.md`](../project/s1-bringup.md); every
command is in [`s1-commands.md`](s1-commands.md); how the software works is
[`docs/s1/design/s1-body.md`](../design/s1-body.md).

**Nothing here is the duck's path yet.** This drives the robot through `netpadd` with datagrams made by `s1-e2e`, and through
`s1ctl`. Neither `duckctl udp-pad` nor the browser has ever run against the S1.

**Rules that are not optional:**

- **No generic camera stream.** `v4l2-ctl --stream-mmap` and `gst-launch v4l2src` rebooted the robot twice. Only `s1-cam`.
- **Never a DDS node reset** (`0x48/0x02`). One left the controller's bus dead until a power cycle.
- **No frame to the robot** without a captured frame or DJI's own library behind it.
- **Wheels off the ground** for `drive`, `items` and anything that moves, unless the step says floor; ask before moving.
- Nothing here writes to `/system` or to a partition. A reboot brings DJI's firmware back and none of ours.

## What it needs

On the Mac: `adb`, `cargo-zigbuild` with `zig`, the Rust target `armv7-unknown-linux-gnueabihf` (`rustup target add`), `ffmpeg`, `python3`; Docker with
`linux/arm/v7` for the rootfs and the V4L2 oracle. In the lab, `~/Development/RoboMaster-S1/out/`, the rootfs tar from `scripts/s1/build-rootfs.sh`, the ONNX Runtime from
`scripts/s1/build-onnxruntime.sh` for stage 12.

| Variable | Default | |
|---|---|---|
| `S1_HOST` | `192.168.1.236:5555` | the robot for `adb`; over USB its serial, `0123456789ABCDEF`, and then set `S1_UDP` too, or its default is built from the serial |
| `S1_BAUD` | none, required to start the daemons | `921600`, measured by `capture` |
| `S1_OUT` | `~/Development/RoboMaster-S1/out/<date>-poc2` | where artefacts land |
| `S1_CHROOT` | `/data/debian` | |
| `S1_UDP` | `<host>:4210` | the robot's `netpadd` |
| `S1_TCP` | `127.0.0.1:9000` | the forwarded socket |
| `S1_VOLUME_CONTROL` | `Speaker` | no valid proof; set empty to pass none |
| `S1_ARGS` | none | extra flags for `s1d`; the only way to pass `--blaster-gel` |
| `S1_ROOTFS` | the lab's tar | `exec` |
| `S1_STAGE`, `S1_PULL_SKIP` | `/sdcard/bk`; `blackbox userdata cache` | `backup` |
| `S1_ORT`, `S1_ORT_VERSION`, `S1_ORT_MAC`, `S1_FRAMES` | the lab's | `infer` |

## From a cold robot

1. **Power on, root adb.** Root `adb` comes from the DJI app's Lab, through `robomaster-s1-hacks` (`enable_root_adb.py`); it has to be enabled again after every power cycle. `adb devices` must list the robot. The owner's hand may be needed here; check first with `ping` and `adb`.
2. **The chroot, once:** `scripts/s1/bench.sh exec`. It pushes and unpacks the rootfs when absent and makes the mounts. A reboot drops the mounts: `drive`, `up`, `camera`, `audio`, `frames`, `infer` and `telemetry-free` make them again; `events`, `items`, `floor` and `capture` do not.
3. **Install, after a build:** `scripts/s1/s1-push.sh`. It builds and installs `s1d`, `s1ctl`, `s1-cam`, `netpadd`, `robotctl`, `robotd.toml`, `asound.conf`, `asound.state.stock`, and links the eight sounds. It does not install `mediad`, `s1-infer` or `pet-detect`.
4. **Start.** Both ways start `s1d` and `netpadd` by hand, and nothing supervises them.
   - `scripts/s1/bench.sh stop`, then `S1_BAUD=921600 scripts/s1/bench.sh drive`, which runs the items. **`drive` does not stop DJI's process: without `stop` first, `s1d` writes to a UART DJI's program still holds.** Proved on 2026-10-30, wheels in the air, in the form that existed then.
   - `S1_BAUD=921600 scripts/s1/bench.sh up` stops DJI's body process and its monitor itself, starts the daemons, forwards the socket and checks four mixer controls against `deploy/s1/asound.state.stock`. **No valid proof:** its only run was the discarded floor session.
5. **Items, wheels in the air:** `scripts/s1/bench.sh items`, or some by name (`items F2 F10`). The prompts need a terminal; without one an item counts as `unwitnessed` and fails.
6. **Floor:** `scripts/s1/bench.sh floor`, 1.5 m free. **No valid proof.** Read the [defects](s1-commands.md#open-defects) first: `robot.stop` does not cancel `s1.goto`.
7. **By hand:** `s1ctl --tcp 127.0.0.1:9000 <command>` from the Mac; on the robot, `robotctl` in the chroot:
   `adb shell /system/xbin/busybox chroot /data/debian /usr/bin/env PATH=/usr/bin:/bin /opt/robot/bin/robotctl health`
8. **Off:** power off, or `scripts/s1/bench.sh restore` to reboot into DJI's firmware and remove the forward. **Restoring a partition image has never been tried.**

## The stages

`scripts/s1/bench.sh help` lists them. Each leaves artefacts under `$S1_OUT/<stage>/`. A re-run moves the earlier run aside only where the stage wrote `cmd.txt`; `items`, `floor`, `e2e` and `app` never do, and overwrite. The status of each is in [the record](../project/s1-bringup.md#the-bench-stage-by-stage).

| Stage | Needs | What it does |
|---|---|---|
| `capture` | stock alive, the app in hand | static facts, three straces for 150 s while the owner follows the script |
| `backup` | stock alive | dumps every partition to the card, pulls all but the three big ones, compares hashes |
| `exec` | root adb | runs a binary from `/data`, unpacks the chroot |
| `stop` | root adb | stops `dji_hdvt_uav` and `dji_monitor`, lists who holds the UART |
| `drive`, `items`, `events` | installed, wheels in the air | the items, with a person answering |
| `camera` | installed | stops `dji_camera`, runs `s1-cam`, makes a PNG |
| `audio` | chroot | stops `dji_camera`, records 60 s, keeps both channels, the left is the microphone |
| `app` | stock stopped, app in hand | notes. Never run |
| `restore` | root adb | reboots; root adb by hand again |
| `frames` | chroot | **records a refusal and measures nothing**; the rate is `camera`'s |
| `infer` | frames, library | `s1-infer` on the robot and on the Mac. CPU per frame is not reliable |
| `telemetry`, `telemetry-gimbal` | stock alive | the stock's subscription; the gimbal's push |
| `telemetry-free` | installed, wheels in the air | head sweep and a tilt by hand, judged by `telemetry-free.py` |
| `up`, `floor` | installed | see above. No valid proof |
| `e2e` | no robot | `s1-e2e fake`: every item on a fake body, judged on the bytes |
| `verify-record <md>` | no robot | every `artefato:` line exists with its hash; says nothing about the sentence |

## With no robot

```
cargo test -p s1-duml -p s1d                 # includes s1-e2e fake, 42 of 42
scripts/s1/bench.sh e2e                      # the same, keeping the bytes
scripts/s1/bench-selftest.sh                 # the stages against a stand-in adb, except items, infer, restore, app
scripts/s1/e2e-refute.sh                     # a planted byte must fail; also telemetry-, audio-, cam-refute.sh
scripts/s1/duml-strace.py --selftest         # also telemetry-free.py --selftest
python3 scripts/s1/docs-check.py             # these pages against the code
```

The refutation scripts use `sed -i ''` and run only on macOS. CI does not build or test the S1.

To rebuild the oracle hex in `s1-duml/tests/dji_oracle.rs`, `scripts/s1/dji-oracle.py` needs DJI's Python library, pulled from a robot; it stays out of git. `scripts/s1/v4l2-oracle.sh` regenerates `s1d/tests/fixtures/v4l2_arm32.json` in Docker.
