# `s1d` and the S1's body

Status: describes the code at `27d4b6e`. Owner of: how the RoboMaster S1 runs the microduck software
today, and which software touches which part of the robot. The dated evidence is
[`s1-bringup.md`](../project/s1-bringup.md); every command is
[`s1-commands.md`](../robot/s1-commands.md); how to run it is [`s1-bench.md`](../robot/s1-bench.md).

**Where this stands.** The code below has tests. On the robot it ran on 2026-10-30 in an older form, with the
wheels in the air, and the newer parts ran only in a floor session the owner discarded. Nothing has run through
`duckctl udp-pad` or the browser. So a sentence here says what the code does, not that the robot does it;
`s1-commands.md` says which is proven.

## 1. The shape of it

```text
 the Mac or the LAN          the robot: Android 4.4, kernel 3.10, ARMv7, DJI's services still installed
 ──────────────────          ───────────────────────────────────────────────────────────────────────────
                             Debian 12 armhf chroot at /data/debian, started by hand with bench.sh
 s1-e2e, duckctl udp-pad ──UDP:4210──► netpadd ──┐
 s1ctl --tcp, s1-e2e ──adb forward──────────────►├─► /run/robotd.sock ─► s1d ─► /dev/ttyS3 921600 ─► motion controller
                                        robotctl─┘                       │
                                        s1ctl ───┘                       ├─ aplay, amixer, alsactl (Debian's)
                                                                         └─ /dev/input/event0 (the button)
 not in this picture, run separately: s1-cam (camera to files), arecord, pet-detect, s1-infer
 not on the robot at all: mediad, the browser's page
```

`s1d` is a separate program from `robotd`, on the same socket path, speaking the same contract for the part
of it a wheeled body can honour. It shares no code with `robotd` or `duck-control`. `s1-duml` is the wire protocol and
does no I/O. `duck-ipc-proto` is reused unmodified; `s1d`'s own list (`s1.*`) lives beside it in `s1d/src/api.rs`.

Nothing restarts a program that dies: `bench.sh` starts `s1d` and `netpadd` with `setsid nohup` (`scripts/s1/bench.sh:139`, `:143`), while
`netpadd` exits on failure and counts on a supervisor (`netpadd/src/robotd.rs:3`). A reboot brings back DJI's services and
none of ours, which is the only recovery path exercised so far.

## 2. The loop

One thread, 100 Hz by default (`--hz`), in `s1d/src/tick.rs`. In order, each tick: takes a snapshot of the
intents; applies a pending arm or disarm; if armed, sends the heartbeat, one wheel or follow frame, and a gimbal frame when due;
armed or not, asks for telemetry if nothing flows, sends a pending LED frame and up to four queued one-shots, runs the blaster's flash and sound;
reads what the body sent, decodes it, publishes state and events.

**The heartbeat is the permission to move.** `0x3f/0x60` on every armed tick. It stops when disarmed, and it
stops if the loop dies. The bytes are proroklab's; DJI's own are built and have never been sent.

**Deadman.** A twist or wheel speeds older than 500 ms (`--deadman-ms`) become zero, with the heartbeat
still on. A gimbal rate ends the same way. An angle target has no deadman: it holds.

**Who owns the wheels on each tick**, first match wins (`s1d/src/tick.rs:512`): a position task sends nothing; lifted
sends zero; fresh `s1.wheels` sends those rpm; `gimbal_lead` sends the twist as `0x3f/0x22`; otherwise the
twist through mecanum kinematics as `0x3f/0x20`. `--twist speed` would send `0x3f/0x21`; it has never run.

**Frames.** The duck's signs are converted where the wheels and the gimbal meet the wire: the duck's +yaw is left, DJI's is right.
`s1.*` keeps DJI's signs. `robot.state` uses the duck's frames.

**Lift rule.** A detector compares the controller's fused velocity with what the four wheel speeds imply. Past
the trip level for 300 ms it zeroes the wheels and sets `safety.picked_up`; gimbal, sound and lights go on. **The levels
have no valid origin** (`s1d/src/tick.rs:248-259`).

## 3. Telemetry: what the robot actually gives

Our own subscription is refused by the motion controller, cause undetermined. `s1d` still sends it, and re-asks only
while nothing flows. **Never a node reset:** one `0x48/0x02` left the bus dead until a power cycle.

What `s1d` reads is what the stock firmware asked for at boot, which the controller keeps pushing after the stock stops:

- `0x48/0x08` of 161 bytes, every 20 ms, decoded by length under any message id (`s1_duml::dds::StockPush`): time, velocities, wheel speeds and angles, battery, position, IMU, status word, attitude. Battery and wheel blocks come in either order between boots, read off the bytes. Each push is answered with the stock's ack.
- `0x04/0x05`, the gimbal's own, about 21 to 39 Hz (`s1_duml::events::GimbalPush`): pitch, ground yaw, yaw relative to the chassis, quaternion. `robot.state.head` is the relative yaw.

**So telemetry depends on the stock having run at boot.** A boot without DJI's program has not been tried, and if nothing subscribes,
this page's section falls: it is the first thing that could refute the target below.

`robot.health` and `robot.state` map those pushes to the contract; `s1.telemetry` gives them in DJI's terms. `safety.limp` is wrongly "disarmed" (`s1d/src/state.rs:356`).

## 4. Camera, audio, inference: outside `s1d`

- **Camera.** `s1-cam` (`s1d/src/cam.rs`, `s1d/src/v4l2.rs`, `s1d/src/bin/s1-cam.rs`) replays DJI's 27 steps and holds every answer to the recording; a difference stops it before streaming. A generic stream rebooted the robot twice, so `v4l2src` and `v4l2-ctl --stream-mmap` are forbidden here. Output: NV12 1280×720 plus 32 trailing bytes, to files. `s1d` does not open the camera, and there is no live output.
- **Speaker.** `s1d/src/sound.rs` runs `aplay` on a bank of links to DJI's wavs. `s1.volume` runs `amixer`. Both need the chroot's PATH.
- **Mixer.** `s1d --alsa-state` runs `alsactl restore` at start (`s1d/src/alsa.rs`). The file was saved during the discarded session. What a boot without the stock needs is unknown.
- **Microphone.** No route in `s1d`. `deploy/s1/asound.conf` defines `micleft`, the left channel at 16 kHz mono, for `arecord` and `pet-detect` by hand.
- **Inference.** `s1-infer` measures the detector's ONNX path. Not wired to anything.
- **Button.** A thread reads `/dev/input/event0` and reports presses as `s1.event` (`s1d/src/button.rs`).

## 5. What is reused from the duck

Read from the code; "ran" means on the robot on 2026-10-30, with datagrams from `s1-e2e`.

| The duck's piece | On the S1 | State |
|---|---|---|
| `duck-ipc-proto`, JSON lines on `/run/robotd.sock` | `s1d` answers the part in [`s1-commands.md`](../robot/s1-commands.md) | reused; has no blaster, armour or gimbal mode |
| `netpadd` + `pad-map` | same code, `[pad_roller]` opened for the strafe in `deploy/s1/robotd.toml` | ran; `duckctl udp-pad` never did; panel alarms on `limp` |
| `robotctl` | same code | ran in part. Duck's texts: `zero3 (not declared)`, "standing up", `systemctl restart`, a duck in 3D |
| `mediad` and its page | **not installed** | never ran. Would need: a source, NV12 or UYVY decided, `webrtcsink`, `--rotate 0`, a menu for this body, answers to `hello` and `system.info` |
| `robotd`'s voice rules (`aplay` per tag, held wheee) | rewritten in `s1d/src/sound.rs` | ran, with DJI's wavs |
| `pet-detect` | by hand through `micleft` | pipeline runs; the duck's model never fires here (0.31 at most) |
| `duck-detect` ONNX path | through `s1-infer` | 1740.9 ms a frame; the duck runs at two a second |
| Bluetooth tools (`btd`, `duck-ble`, `duckctl open`, `ip`, `ssh`) | not used | Bluetooth on the S1 not measured |
| Walking policy, depth sensor, mouth | do not apply | no legs, no sensor, no mouth |
| `kinematics`, `pad-imu` | compiled in, no function here | pulled by `robotd-params` and `pad-map` |

<a id="every-part-by-component"></a>

## 6. Every part, by component

Table sent to the owner on 2026-11-01, in Portuguese, as sent, with two corrections: it listed `gimbal_degree` for H41 and `gimbal_subscribe_position` for H43,
and `s1d` calls neither (`git grep`: only their definitions). Which stage proved each part is in
[`s1-bringup.md`](../project/s1-bringup.md#every-part-of-the-robot-by-stage).

| Peça | Quadro (`s1-duml`) | Programa do robô (`s1d`) | Controle (`duckctl` → `netpadd` + `pad-map`) | `robotctl` | Console (`mediad` + navegador), por leitura | Bancada |
|---|---|---|---|---|---|---|
| H5 Serial | `frame`, `crc` | `body.rs` | todo comando passa aqui | todo comando passa aqui | nunca rodou | `duml-strace.py`, `bench.sh capture` |
| H6 Wi-Fi | — | — | transporta o UDP; sem gerência | — | transportaria o vídeo | `bench.sh` por `adb` |
| H12 Botão de programa | — | `button.rs`, `s1.event` | sem botão | — | sem botão | `s1ctl events` |
| H20 Câmera | — | `cam.rs`, `v4l2.rs`, `s1-cam` | — | `frame` recusa não-UYVY (`robotctl/src/frame.rs:63-65`) | vídeo exige fonte nova, formato e `webrtcsink` | `bench.sh`, `cam-refute.sh` |
| H21 Microfone | — | nenhuma rota | — | — | sem áudio | `deploy/s1/asound.conf`, `arecord`, `pet-detect` à mão |
| H22 Alto-falante | — | `sound.rs`, `robot.sound` | RT e LT tocam no **laptop**, salvo `--robot-sound-on` (`duckctl/src/udp_pad.rs:418-422`); `s1-e2e` toca no robô | `quack` | 7 sons (`mediad/webclient/index.html:305-311`) | `bench.sh audio` |
| H23 Volume | — | `s1.volume` | sem botão | — | sem botão | `s1ctl volume` |
| H24 Mixer | — | `alsa.rs` | — | — | — | `deploy/s1/asound.state.stock` |
| H30 Armar | `heartbeat_proroklab`, `chassis_workmode` | `robot.enable`, `robot.init`, `robot.relax` | Start | `robot enable/init/relax`, com texto do pato | botões `enable`, `init`, `relax` | `s1-e2e` |
| H31 Rodas | `chassis_wheel_speed`, `chassis_position`, `chassis_speed` | `robot.move`, `s1.wheels`, `s1.goto` | sticks; lateral só no modo Drive | sem comando | WASD/QE, até 0,3 m/s e 1,5 rad/s | `s1ctl wheels/goto` |
| H33, H34 IMU e odometria | `dds` | `state.rs`, `s1.telemetry`, `robot.state` | painel | `monitor`, com pato em 3D | `robot.subscribe` a 2 Hz | `telemetry-free.py` |
| H35 Tombado, levantado | `dds` | `tick.rs`, `safety` | painel | `monitor` | aparece como alerta | `telemetry-free.py` |
| H36 PWM | `chassis_pwm_value`, `chassis_pwm_freq` | `s1.pwm` | sem botão | — | sem botão; 0 de 14 `s1.*` | `s1ctl pwm` |
| H40 Gimbal por ângulo | `gimbal_angles`, `gimbal_angles_axes` | `robot.head`, `robot.look` | modos D-pad cima e direita; modo Head usa juntas que o gimbal ignora | `robot look` | arrastar na imagem (`mediad/webclient/index.html:1700`) | `s1-e2e` |
| H41 Recentrar, velocidade, sono | `gimbal_recenter`, `gimbal_speed`, `gimbal_suspend_resume` | `s1.gimbal` | sem botão | — | sem botão | `s1ctl gimbal` |
| H42 Modos de seguir | `gimbal_workmode`, `chassis_follow`, `chassis_stick_overlay` | `s1.follow` | sem botão | — | sem botão | `s1ctl follow` |
| H43 Ângulo lido | `events::GimbalPush` | `state.rs` | painel | `monitor` | `robot.state` | `telemetry-free.py` |
| H44, H52 LEDs | `system_led` | `robot.flashlight`, `s1.lights` | Home | `flashlight` | só pelo campo de texto; sem botão | `s1ctl lights` |
| H45, H46, H50 Golpes | `events` | `s1.event` | — | — | sem tela | `s1ctl events` |
| H51 Sensibilidade | `armor_sensitivity` | `s1.armor` | — | — | sem botão | `s1ctl armor` |
| H60, H61, H62 Lançador | `gun_fire`, `gun_fire_flash`, `gun_led` | `s1.blaster`; gel recusado | sem botão | — | sem botão | `s1ctl blaster` |
| H70 Bateria | `dds` | `robot.health` | painel | `health`, `monitor` | `robot.health` | `telemetry-free.py` |

**Só bancada, sem comando:** H1 CPU, H3 eMMC, H4 microSD, H8 micro-USB como `adb`. H32 rodas mecanum vão com H31.

**Sem nenhum componente:** H2 RAM, H7 hub interno, H9 micro-USB como host, H10 porta reservada, H11 Bluetooth, H13 chave de modo e botão de conexão, H14 codificador H.264, H15 coprocessador, H16 MCU e CPLD, H37 portas reservadas, H38 LED da placa, H71 desligar por software, H72 LEDs da bateria. H39 e H63 não se alcançam por software, pelo manual.

## 7. The target architecture (proposed, not accepted, not proven)

**This section is a proposal.** The owner has given a direction on it, on an earlier diagram; this version has not been approved, and nothing of it has run.

**The skeleton:** each robot keeps its own program behind the same socket. The S1 does not become a `robotd` configuration, and shared pieces stay
shared. A browser and a pad work on both bodies. A conformance battery runs the same contract tests against `robotd --fake` and `s1d --fake`.

| Piece | Change | Touches the Pollen's code? |
|---|---|---|
| `mediad`, source | a new source without `v4l2src`, fed by the S1's camera sequence | yes |
| Format | convert NV12 to UYVY at the source, or parametrise; UYVY is fixed in six files. Decided by measuring the CPU | no if converting, yes if parametrising |
| `mediad`, permission | a way for `s1.*` through (`mediad/src/session.rs:218`) | yes, one place |
| The page | show what this body has, no duck menu | yes |
| `hello`, `system.info` | answered by `s1d`, through `mediad`'s `--updater-socket` and `--config-socket` (`mediad/src/main.rs:161-169`) | no |
| Rotation | `--rotate 0` | no |
| `s1d` | `limp`, skills, `robot.stop` cancels `s1.goto`, no catch-all arm | no |
| `robotctl` | a subset or adjusted texts, to decide | to decide |
| Chroot | `plugins-bad`, `ugly` or `openh264`, `nice`; `webrtcsink` built for armhf | no |
| Supervisor | something that brings back what falls | no |
| Configuration | a separate file: the editor refuses an `[s1]` section (`robotd-params/src/edit.rs:114`) | no |

**When the Pollen updates:** a new contract method compiles silently in `s1d` today (`s1d/src/ipc.rs:723`); in the proposal `s1d` stops compiling until someone decides.
`netpadd`, `pad-map` and `robotctl` arrive on both. `robotd`, `duck-control` and `odometry` arrive only on the duck, and nothing warns.

**What would refute it, none measured yet:**

- software H.264 with `s1d` running bringing the 100 Hz loop down;
- `webrtcsink` not building or not running on armhf with GStreamer 1.22;
- no telemetry at all with DJI's program absent at boot, and no other source.

**Open, and the owner's:** where the S1's commands live, in the duck's list or beside it (recommended here: beside, one point in `mediad`, against four matches in
`duck-ipc-proto/src/lib.rs` and two transports per command; refuted if that one point lets a command through without a permission, or if the page needs types only the duck's list has);
authentication, since every `s1.*` opened in the browser also opens from outside if there is a login (`mediad/src/relay.rs`); what becomes of DJI's firmware functions.
