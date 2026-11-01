# The S1, command by command

Every call `s1d` answers on its socket, what it puts on the wire, what tests it, what has proved it on
the robot, and how it can be reached today from the controller, from `robotctl`, from `s1ctl` and from
the browser. This page owns that list; it is the base for wiring commands to the console and the pad.
`scripts/s1/docs-check.py` fails when a call exists in the code and not here, or here and not in the code.

Read from the code at `27d4b6e`, with no robot touched. The dated evidence is in
[`docs/s1/project/s1-bringup.md`](../project/s1-bringup.md); paths like `drive/e2e.log` are under
`~/Development/RoboMaster-S1/out/2026-10-30-poc2/`. How the body works is
[`docs/s1/design/s1-body.md`](../design/s1-body.md); how to run it is [`s1-bench.md`](s1-bench.md).

**How to read "on the robot".** "Seen, in the air" means the owner watched it on 2026-10-30 with the wheels
off the ground, and `drive/e2e.log` holds the transcribed witness. `items/e2e.log` holds what the daemon answered and the events, with every witness `unwitnessed` and every result `FAIL`, since it ran with no terminal: where only the record's prose says the owner saw something, the cell says so. "No valid proof"
means the only robot run was the floor session of 2026-10-31, which the owner discarded, or there was none.
**No call here has been proved through the duck's path**, `duckctl udp-pad` and the browser. The nearest
is "through `netpadd`", with the datagram built by `s1-e2e`.

**The browser column is read from the code; none of it has run on the S1.** `mediad` is not installed
by `s1-push.sh`, has never been built for this CPU, and refuses any `s1.*` (`mediad/src/session.rs:218`:
`as_call` does not know the name).

## How a call is routed

- `s1.*` is matched first (`s1d/src/ipc.rs:202`), then everything else parses as the contract's `proto::Call`.
- A call without an id is a notification and gets no reply. Only `robot.move`, `robot.head` and `robot.sound`
  do anything as notifications (`s1d/src/ipc.rs:468-482`); `s1.*` without an id is ignored (`s1d/src/ipc.rs:203`).
- The socket is mode 0660 and has no per-method permission. Over the network nothing reaches it today except
  `netpadd`'s UDP port, which takes no authentication (`netpadd/src/receiver.rs:147-160`).
- **What needs the robot armed** (`s1d/src/tick.rs:469`): the heartbeat, wheel and follow frames, gimbal by
  angle and by rate. **What goes out armed or not:** the telemetry subscription, LEDs, the queue of one-shot
  frames (at most four a tick, no limit on its length, `s1d/src/intents.rs:273`), the blaster's flash and sound.

## `robot.*` the body serves

| Call | What `s1d` does | On the wire | Test | On the robot | Controller (`duckctl` → `netpadd`) | `robotctl` | Browser | Missing for the duck's path |
|---|---|---|---|---|---|---|---|---|
| `hello` | answers the contract's version and its own | — | **none:** the route test skips it, since its destination is the updater | answered | — | on connect (`robotctl/src/main.rs:1548`) | asked at open, but routed to `updaterd`, absent here: red alert (`duck-ipc-proto/src/lib.rs:1420`, `mediad/webclient/index.html:1302`) | an answer for the console |
| `robot.move` | sets the twist, under the deadman | armed: `0x3f/0x20` each tick by mecanum; `0x3f/0x22` in `gimbal_lead`; `0x3f/0x21` under `--twist speed`, never sent | `the_deadman_zeroes_the_wheels_and_keeps_the_heartbeat` (`s1d/tests/fake_body.rs:23`), fake run | seen, in the air, through `netpadd` (`drive/e2e.log`). On the floor: no valid proof | left and right stick; strafe only in Drive mode | no command | WASD/QE, up to 0.3 m/s and 1.5 rad/s (`mediad/webclient/index.html:1510`) | real `duckctl`; browser; floor; scale never measured |
| `robot.head` | sets the head target in the duck's frame, converted to DJI's signs | armed: `0x3f/0xb0`, coordinate 3, re-sent every 100 ms; once per change in `gimbal_lead`; pitch only in `chassis_lead` | `the_head_target_is_sent_once_where_the_gimbal_leads_and_held_elsewhere` (`s1d/tests/fake_body.rs:576`) | ±60° to 0.1° in free mode, through the socket (`telemetry-free/verdict.txt`); seen through `netpadd` (`drive/e2e.log`) | D-pad up, right and down modes, and a zero head on leaving them (`pad-map/src/mapper.rs:51`); Head mode moves axes a gimbal ignores; yaw up to ±143° (`robotd-params/src/pad_axes.rs:252-256`) against the manual's ±250° | no command | not used; the console uses `robot.look` | real `duckctl`; the follow modes have no valid proof |
| `robot.look` | turns a point in the trunk frame into two gimbal angles, same slot as `robot.head`, answers `LookResult` | as `robot.head` | `look_answers_a_look_result_and_aims_the_gimbal` (`s1d/tests/dispatch.rs:127`) | no valid proof | — | `robot look` (`robotctl/src/main.rs:3453`) | drag on the picture (`mediad/webclient/index.html:1700`); depends on video | robot run; browser |
| `robot.stop` | zeroes twist and wheel speeds | the next wheel frame is zero | fake run item F16 | seen as part of F16, in the air | none: the pad stops by centring the sticks | no command | `stop` button (`mediad/webclient/index.html:283`) | **does not cancel `s1.goto`** (see [open defects](#open-defects)) |
| `robot.enable` | arms or disarms; `toggle` flips; answers `armed` or `disarmed` as the reason | arm: five `0x3f/0x19 01`, then `0x04/0x4c 00 00`; then `0x3f/0x60` each tick. Disarm: wheels zero, heartbeat off | `enable_toggles_and_arms` (`s1d/tests/dispatch.rs:43`), `arming_frees_the_gimbal_from_the_chassis_once` (`s1d/tests/fake_body.rs:463`) | through the socket, armed and disarmed with the wheels obeying (`drive/move.log`, the owner's word in the record). The pad's first Start is `robot.init`; its toggle is witnessed only in the record's prose. The bytes are proroklab's; the stock's heartbeat was never sent | Start, after the first press (`pad-map/src/mapper.rs:253`); Start held sends off, then `robot.init` (`pad-map/src/mapper.rs:273-277`), which here disarms and re-arms | `robot enable` (`robotctl/src/main.rs:3429`) | `enable` button (`mediad/webclient/index.html:280`) | real `duckctl`; browser |
| `robot.init` | arms | as `robot.enable` | `every_robot_route_is_served_or_refused_with_a_reason` | first Start press in the pad item | Start, first press (`pad-map/src/mapper.rs:240`), and Start held; the log says "standing up" | `robot init`, with the duck's text | `init` button | texts |
| `robot.relax` | disarms | wheels zero, heartbeat off; does not stop `s1.goto` | `every_robot_route_is_served_or_refused_with_a_reason` | no witness of its own | — | `robot relax --yes`; without the flag it refuses, with the duck's "the robot will collapse" (`robotctl/src/main.rs:3417`) | `relax` button | defect, below |
| `robot.health` | loop rate, battery, healthy or not | — | `health_before_the_first_tick_is_not_healthy_but_answers` (`s1d/tests/dispatch.rs:69`) | `drive/health.json`: healthy, 99.98 Hz | `netpadd` health (`netpadd/src/health.rs`) | `health`, `monitor`; says `zero3 (not declared)` (`robotctl/src/main.rs:2168-2177`) | polled (`mediad/webclient/index.html:1376`) | body name in `robotctl` |
| `robot.safeToRestart` | safe unless a non-zero, fresh twist is applied | — | route test only | not exercised | — | — (asked by `updaterd`, absent) | not permitted (`mediad/src/route.rs`) | ignores `s1.wheels` and `s1.goto`: defect |
| `robot.modelApi` | answers `s1d`'s model API number | — | route test only | not exercised | — | — | not permitted | nothing asks it on the S1 |
| `robot.remoteSessionActive` | always `{"active":false}` | — | route test only | not exercised | — | — | not permitted | a real answer if `mediad` comes |
| `robot.mode` | always `roller` | — | `mode_is_roller` (`s1d/tests/dispatch.rs:19`) | answered (`s1-bringup.md`, stage 5) | `netpadd` asks it to pick `[pad_roller]` (`netpadd/src/robotd.rs`) | `robot mode` | polled (`mediad/webclient/index.html:1372`) | nothing |
| `robot.policies` | `roller`, empty slots and skills | — | route test only | through `netpadd`, indirectly | `netpadd/src/main.rs` | policy commands, with the duck's text | fills the skill menu; empty makes it offer the duck's sit, kick and roll (`mediad/webclient/index.html:1400-1417`) | **the S1's actions as skills** |
| `robot.subscribe` | starts `robot.state` on this connection at `hz`; answers `unavailable` + no skills | — | **no test in `s1d/tests`;** only `s1-e2e` (`s1d/src/bin/s1-e2e.rs:1174`). What fills the state: `stock_pushes_fill_health_state_and_the_head` (`s1d/tests/fake_body.rs:207`) | `robotctl monitor`, 2026-10-30; `safety.fallen` proven; `safety.limp` is wrong | panel | `monitor`, drawing a duck with 15 joints | 2 Hz (`mediad/webclient/index.html:1347`) | `limp` fixed |
| `robot.flashlight` | solid colour or off on the body LEDs, `toggle` inverts the last call | `0x3f/0x32` to `0x18` | `flashlight_toggle_inverts_what_on_and_off_left` (`s1d/tests/dispatch.rs:100`) | seen through `netpadd`: Home, white on and off (`drive/e2e.log`) | Home | `flashlight` (`robotctl/src/main.rs:647`) | no button; text field only | real `duckctl`; a button |
| `robot.sound` | plays a tag of the bank through `aplay`; `hold` makes wheee a level; refused without `--sounds` | none: speaker through ALSA | unit test in `s1d/src/sound.rs`, fake run | heard through `netpadd` ("Sim, ouvi os dois", `drive/e2e.log`) and by `robotctl quack` | RT chirp, LT wheee (`pad-map/src/mapper.rs:406`), **but real `duckctl` plays them on the laptop unless `--robot-sound-on`** (`duckctl/src/udp_pad.rs:418-422`) | `quack` (`robotctl/src/main.rs:621`) | 7 tags (`mediad/webclient/index.html:305-311`) | real `duckctl`; browser; the bank is DJI's wavs under the duck's names |
| `robot.skills` | empty | — | route test only | not exercised | — | `robotctl/src/main.rs:3856` | — | **the S1's actions as skills** |
| `pad.bindings` | empty | — | route test only | not exercised | — | — | permitted (`mediad/src/route.rs:500`) | the same |

**The bank** (`scripts/s1/s1-push.sh`), tag → DJI's file under `/system/audio/sound`: chirp `media_sound_demo_recognize_success`,
greet `power_on`, alarm `media_sound_demo_attacked`, inquire `media_sound_demo_scanning`, peck `sleeping`, coo
`media_sound_demo_gimbal_rotate`, wheee `engine_start`, and `shoot` `ir_gun_shoot`, which only `s1.blaster` plays.

## `robot.*` the body refuses with a reason

The call exists in the contract and this body cannot honour it: the answer is `IntentResult { accepted: false, reason }`.
`every_robot_route_is_served_or_refused_with_a_reason` (`s1d/tests/dispatch.rs:80`) proves only that a route does not fall to `METHOD_NOT_FOUND`, not the reason or the shape, and wherever this page says "route test only" that is all there is. Its list, `every_call`, leaves out 6 of the contract's 79 variants: `robot.chorale`, `chorale.subscribe`, `robot.setMode` and `robot.theremin` of this table, so nothing exercises those four, and `chorale.beacon` and `chorale.heard`. `robot.do` is also held by `do_is_refused_with_not_driving` (`s1d/tests/dispatch.rs:29`).

| Call | Reason given today | Who would send it on the S1 | What it would take |
|---|---|---|---|
| `robot.do` | the policy is not driving; no skills | any pad button bound to a skill (`pad-map/src/mapper.rs:325`; by default A `sit_toggle`, B `ground_pick`, since `deploy/s1/robotd.toml` has no `[pad]`), X held if bound (`pad-map/src/mapper.rs:352`), pad loss (`pad-map/src/loss.rs:123`), `robotctl robot do`, console `do` | **the way to plug blaster, follow, recentre and sleep into both clients without touching the contract**, with `robot.policies`, `robot.skills` and the `[pad]` section |
| `robot.pose` | no legs | pad Body mode, D-pad down (`pad-map/src/mapper.rs:524`), and on leaving it (`pad-map/src/mapper.rs:45`) | not applicable to the body |
| `robot.mouth` | no mouth | pad triggers, as notification, ignored | not applicable |
| `robot.rest` | neither sits nor powers off | pad Select held 2 s (`pad-map/src/mapper.rs:375`) | power-off, see below |
| `robot.shutdown` | the same | pad Select held 4 s (`pad-map/src/mapper.rs:387`); console button with a confirmation (`mediad/webclient/index.html:284`) | **the power-off frame, undecoded in `capture-follow/sys_poweroff.strace`** |
| `robot.setMode` | always on wheels | no client on the S1 | not applicable |
| `robot.theremin` | no depth sensor | `robotctl` | not applicable |
| `robot.chorale` | no voice | `robotctl` | not applicable until there is a voice |
| `chorale.subscribe` | the same | — | the same |
| `robot.model` | no kinematic model | — | phase 5 of the old plan |
| `robot.vetbook` | keeps no vet book | `robotctl vetbook` | the owner's decision |
| `robot.vetbookNote` | the same | `robotctl` (`robotctl/src/vetbook.rs:161`) | the same |
| `robot.vetbookTally` | the same | — | the same |
| `robot.loadPolicy` | no policy network | `robotctl` | not applicable |
| `robot.reloadPolicies` | the same | `robotctl` | not applicable |
| `robot.setSkill` | the same | no client on the S1 | with the skills above |
| `robot.removeSkill` | the same | no client on the S1 | with the skills above |
| `pad.bind` | the same | no client on the S1 | with the skills above |
| `robot.rebootMotors` | motors are behind the motion controller | `robotctl` | not applicable |

## `s1.*`, the S1's own list

Served beside the contract on the same socket (`s1d/src/api.rs`, `S1_API_VERSION` 1). Parameters refuse unknown members by name
(`an_unknown_params_member_is_refused_by_name`, `s1d/tests/s1_api.rs:40`). `s1.gimbal` and `s1.telemetry` use DJI's signs: yaw positive right, pitch positive up, the opposite of `robot.head`. **`s1.goto` is not settled.** The frame carries `-yaw` and `y` as given (`s1d/src/ipc.rs:343`, `s1d/src/ipc.rs:351`). DJI's library, read and not measured, has a positive yaw on the wire as anticlockwise (`rm_ctrl.py:627-630`) and a positive `y` as right (`rm_ctrl.py:1020-1021`), so a positive `s1.goto` yaw would be clockwise and a positive `y` right: DJI's signs, not `robot.move`'s. No valid run decides it. Do not wire a button to it before one does.

**Reach:** `s1ctl` reaches all of them, on the robot or through a forwarded port. **The controller reaches none, and neither does the browser:**
0 of 14 go through `mediad`. How they should reach the browser is an open decision of the owner.

| Call | What it does | On the wire | Test | On the robot | `s1ctl` | Missing for the duck's path |
|---|---|---|---|---|---|---|
| `s1.follow` | who leads: `free`, `gimbal_lead`, `chassis_lead`; `velocity_frame`; `gimbal_fpv` | `0x04/0x4c` + `0x3f/0x28`; then `robot.move` becomes `0x3f/0x22` in `gimbal_lead`. **Arming afterwards sends `0x04/0x4c 00 00` and keeps the mode in the slot** (see defects) | `follow_sets_the_mode_and_queues_gimbal_mode_and_velocity_frame` (`s1d/tests/s1_api.rs:175`), `queued_frames_and_gimbal_lead_reach_the_wire` (`s1d/tests/fake_body.rs:271`) | **no valid proof**: accepted in the air (`items/e2e.log`), not observable. Bytes are DJI's (`capture-follow/hdvt_follow.strace`), except that the app repeats mode 2 at 8 Hz and `s1d` sends it once | `follow` | floor; button; route |
| `s1.gimbal` | `recenter`, `speed`, `stop`, `sleep`, `wake` | `0x3f/0xb2`; `0x04/0x0c` every tick while fresh, armed only; one zero rate on `stop` or at the deadman; `0x04/0x0d`. **While armed the angle hold keeps sending `0x3f/0xb0` over `recenter` and `sleep`** (see defects) | `gimbal_calls_queue_the_librarys_frames` (`s1d/tests/s1_api.rs:80`), `a_gimbal_rate_is_streamed_until_stopped_and_the_angle_hold_waits` (`s1d/tests/fake_body.rs:636`) | **no valid proof**, none of the five | `gimbal` | robot run; button; route |
| `s1.goto` | moves a distance, the controller closing the loop; `stop` abandons it | `0x3f/0x25` with an ack at the end; no wheel frame while the task runs, except zeros while lifted, until `stop`, a non-zero `robot.move` or `s1.wheels`, or distance/speed + turn/rate + 2 s, computed from the unclamped distance (see defects) | `goto_queues_the_position_task_and_its_stop` (`s1d/tests/s1_api.rs:131`), `a_position_task_owns_the_wheels_until_a_twist_takes_them_back` (`s1d/tests/fake_body.rs:511`) | **no valid proof** | `goto` | **defects first**; floor; route |
| `s1.wheels` | rpm per wheel, ±1000, under the deadman, armed only | `0x3f/0x20` each tick | `direct_wheel_speeds_reach_the_wire_and_die_with_the_deadman` (`s1d/tests/fake_body.rs:319`) | seen, in the air: F16, 100 rpm (`drive/e2e.log`) | `wheels` | route |
| `s1.lights` | effect, colour, times, groups, or the launcher's LED | `0x3f/0x32` to `0x18`, or `0x3f/0x55` to `0x17` | `pwm_lights_armor_and_blaster_queue_what_the_library_sends` (`s1d/tests/s1_api.rs:197`) | seen: five effects on the whole body and the trail light (`drive/e2e.log`). **Per group: never measured**; flash and marquee numbering from DJI's library | `lights` | groups; button; route |
| `s1.blaster` | fires `times` 1..8, infrared; gel refused without `--blaster-gel` | `0x3f/0x51` to `0x17`, then per shot, 330 ms apart, flash `0x3f/0x55` and `shoot.wav` | `a_volley_lights_the_muzzle_and_sounds_each_shot_on_the_launchers_cadence` (`s1d/tests/fake_body.rs:386`), same test as lights | infrared fired: six hits per three shots off a wall, 330 ms apart (`items/F17_infrared_fire3.log`); flash and sound witnessed. **Gel never fired.** "The tank out" is the reason's text; no software checks it | `blaster` | button; route; authentication |
| `s1.armor` | hit sensitivity `k`, 0.1..5 | `0x3f/0x07` to `0x18`; DJI's library sends it to `0x09` | same test as lights, **which does not prove the frame sent** (`s1d/tests/s1_api.rs:254-260`) | the record's prose says accepted, with no artefact; effect never observed | `armor` | defect; button; route |
| `s1.pwm` | duty and/or frequency on the six ports | `0x3f/0x2b`, then `0x3f/0x3c` | same test as lights | the record's prose says accepted, with no artefact; **never measured with a load** | `pwm` | a load; route |
| `s1.volume` | speaker percent through `amixer sset <control>`; refused without `--volume-control` | none | `wheels_volume_version_and_telemetry_answer` (`s1d/tests/s1_api.rs:301`), stand-in in the fake run | **no valid proof** | `volume` | robot run; button; route |
| `s1.telemetry` | what the controller pushes: wheels, odometry, velocities, IMU, attitude, status word, gimbal angle and quaternion, battery, acks | none; reads the stock's boot-time push | `the_stocks_push_and_ours_both_become_telemetry` (`s1d/tests/fake_body.rs:136`), `s1-duml/tests/stock_push.rs` | **accepted by the owner, 2026-10-30** (`telemetry-free/verdict.txt`, `pickup.ndjson`). `sticks` arrives only under our refused subscription, so it stays empty (inferred). **Depends on the stock having subscribed at boot** | `telemetry` | a boot without the stock; route |
| `s1.subscribe` | turns `s1.event` on or off for this connection | none | only the fake run's events item; `an_armour_hit_on_the_wire_becomes_an_event` (`s1d/tests/fake_body.rs:357`) tests the event, not this call | `items/e2e.log` | `events` | a screen; route |
| `s1.event` | notification: `armor_hit`, `ir_hit`, `button` | reads `0x3f/0x02`, `0x3f/0x10`, `/dev/input/event0` | the same, fake run | 23 hits, plates 1 to 4; infrared; button codes 264 and 265, which button is which not mapped. Plates 5 and 6: never seen | `events` | a screen; route |
| `s1.frame` | one raw frame, as hex, unfiltered: it would send the DDS node reset that kills the bus | as given, with a fresh sequence | fake run F15 | the record's prose says F15 was accepted, with no artefact | `frame` | **must never leave the robot**; block it at any bridge |
| `s1.version` | list and daemon versions | none | `wheels_volume_version_and_telemetry_answer` | called (`items/e2e.log`); seen only in the record's prose | `version` | route |

## Contract calls `s1d` does not name

They answer `METHOD_NOT_FOUND` from the catch-all arm (`s1d/src/ipc.rs:723`). **That arm is itself a risk: a new variant from the Pollen compiles here without
anyone deciding about it** (see defects). `system.info` is one of the two the console asks at open, and nothing answers it here. The other, `hello`, `s1d` does answer, but `mediad` would route it to the updater.

| Call | Group |
|---|---|
| `update.check` | updater |
| `update.apply` | updater |
| `update.rollback` | updater |
| `update.resetToGolden` | updater |
| `update.select` | updater |
| `update.pin` | updater |
| `update.status` | updater |
| `update.listInstalled` | updater |
| `update.log` | updater |
| `update.show` | updater |
| `update.subscribe` | updater |
| `chorale.beacon` | voice |
| `chorale.heard` | voice |
| `policy.check` | policies |
| `policy.install` | policies |
| `policy.fetch` | policies |
| `policy.search` | policies |
| `detector.check` | detector |
| `detector.install` | detector |
| `account.login` | account |
| `account.status` | account |
| `account.logout` | account |
| `vetbook.share` | vet book |
| `net.status` | network |
| `net.scan` | network |
| `net.connect` | network |
| `net.forget` | network |
| `system.info` | system; the console asks it at open (`mediad/webclient/index.html:1327`) |
| `system.services` | system |
| `system.logs` | system |
| `system.setName` | system |
| `system.reboot` | system |
| `system.pairingPin` | system |
| `system.setPairingPin` | system |
| `system.authenticate` | system |
| `pad.status` | pad |
| `pad.pair` | pad |
| `pad.forget` | pad |
| `pad.input` | pad |
| `tof.stream` | no sensor |
| `head_imu.stream` | no sensor |

## What is not a command

The capabilities of the classification of 2026-10-31 that have no line above, with what stands behind each.

| Capability | State | Evidence or reason | Missing |
|---|---|---|---|
| Our binaries run on the robot, in a Debian chroot under `/data` | proven | `exec/exec.txt` | — |
| The UART freed by stopping the stock | proven | `stop/owners.txt` | — |
| DUML frames and CRCs | proven | `s1-duml/tests/real_frames.rs`, frames captured on the robot | — |
| The stock's heartbeat (`--heartbeat stock`) | test only | never sent (`s1-duml/src/cmd.rs:117`) | prove or remove |
| `--twist speed` (`0x3f/0x21`) | test only | never sent (`s1-duml/src/cmd.rs:166`) | prove or remove |
| Wheels on the floor and their real scale | no valid proof | radius from the URDF (`s1d/src/tick.rs:53`) | measure |
| Per-group lights | claim | `s1d/src/api.rs:163-165` says the library's map; not measured | measure |
| The duck's synthesised voice on the speaker | by hand | heard on 2026-10-31; `s1d` plays DJI's wavs, not `sounds` | wire it, if wanted |
| Microphone capture | by hand | owner recognised the voice (`docs/s1/project/s1-bringup.md`, "Item 3, CLOSED"); **`s1d` has no route** | decide |
| Mixer restored on a boot without the stock | test only | `s1d/src/alsa.rs`; never tried | first proof of a boot without the stock |
| Petting detector | by hand, never fires | 0.307 and 0.107 against a 0.95 threshold; not in `s1d` or `s1-push.sh` | retrain and wire |
| Camera to files | proven | `s1-cam`, 30.1 fps, 395 frames, 0 dropped | **no live output** |
| H.264 video over WebRTC | nothing | no code; needs a source, NV12 to UYVY or a parameter, `webrtcsink` for armhf, a software encoder | the whole video front |
| Duck detector on the CPU | by hand | 1740.9 ms a frame; not wired to anything | the owner's decision on a model |
| Our own telemetry subscription | refused by the robot | answers `02 03`, `0f 03` (`items/F17_infrared_fire3.log:1-3`) | another source, if a boot without the stock fails |
| DJI's `picked_up` bit | never set | 0 of 372, 0 of 94 (`telemetry-free/pickup_bits.txt`, `pickup_wheels_bits.txt`) | do not use |
| The "lifted, wheels stop" rule | test only | see defects | measure again |
| Booting without DJI's program | nothing | never tried; telemetry and the mixer depend on it | first proof |
| Autostart and a mode chosen at boot | nothing | owner's decision of 2026-10-29, no code | restore tested first |
| Power-off by software (`s1.power`) | nothing | capture in the lab, not decoded | decode, then a route |
| Backup of the partitions | proven | `backup/SHA256SUMS`, 11 of 11 equal, 14 on the robot | — |
| Restoring a partition image | never done | — | before any autostart |
| A reboot gives DJI's firmware back | seen, no artefact | twice on 2026-10-30 | run `restore` |
| Managed Wi-Fi, MA2100 coprocessor | nothing | no code | later phases |
| USB host, Coral | inference | LED dark twice; no meter reading | measure VBUS |
| Bluetooth | not measured | firmware and `ttyS2` on the system; no code | measure |
| `duckctl udp-pad` against the S1 | never run | — | the proof of the pad path |
| `netpadd` on the robot | in the air | `drive/e2e.log` | real `duckctl` |
| Browser at `:8080` | never run | — | the proof of the console |
| CI for the S1 | nothing | no line in `.github/workflows` mentions it | later |
| The console opening without a red alert | nothing | `hello` and `system.info` have no service here | answer the two |
| A coherent action menu | nothing | see `robot.policies` | skills |
| Pad buttons for blaster, follow, recentre, sleep | nothing | `[pad]` is free text (`robotd-params/src/lib.rs:267-282`); `deploy/s1/robotd.toml` has no `[pad]` | `robot.do` |
| Console with a test picture, before the camera | nothing | `[media] source = "test"` in `robotd.toml` (`robotd-params/src/lib.rs`; the old `camera = false` is ignored, and the default is the camera, which here is forbidden); needs `webrtcsink` | build it |
| `webrtcsink`, encoder and `libnice` in the chroot | nothing | the rootfs has no `bad`, `ugly` or `nice`; `scripts/setup-gstreamer.sh:178` is `aarch64` only | build and measure |
| The picture the right way up | nothing | board default `Zero3` turns it 90° (`robotd-params/src/board.rs:82-87`) | declare 0° |
| A sign that the picture is going out | nothing | `mediad/src/indicator.rs:31` is silent without its LED | the owner's call |
| Audio in the browser | does not exist on the duck either | no audio in `mediad/src/pipeline.rs` | the owner's call |
| Daemons coming back after a failure | nothing | started by hand, no supervisor (`scripts/s1/bench.sh:139`) | a supervisor |
| `robotctl` saying only what is true here | in part | `robotctl/src/main.rs:3504-3505` "standing up", `robotctl/src/configure.rs:227-244` `systemctl restart` | a subset |
| `duckctl open`, `ip`, `ssh`, `scp` | nothing | Bluetooth only (`duckctl/src/main.rs:1530-1537`); `udp-pad <address>` does not need it (`duckctl/src/main.rs:1481-1506`) | another way, or say so |
| Authentication on browser and pad | the owner's decision | none by the Pollen's choice (`mediad/src/main.rs:9-13`); here it includes the launcher | decide |
| DJI's firmware functions | the owner's decision | six AI modules, follow by camera, zoom, photo, autonomous program, skills, games; not parts | decide |
| A configuration of the S1's own that `robotctl configure` accepts | nothing | daemons tolerate a new key; the editor is strict (`robotd-params/src/edit.rs:114`) | a separate file or the registry |

## Open defects

Found reading the code. None is fixed, and none has been reproduced with a test, unless it says so. Fixing them is the code front's.

1. **`robot.stop` does not cancel `s1.goto`.** `stop` calls `set_twist` and `set_wheels` with zeros (`s1d/src/intents.rs:237-240`), and those release the task only when non-zero (`s1d/src/intents.rs:210`, `:220`); while the task lives the loop sends no wheel frame (`s1d/src/tick.rs:513`) and no `chassis_position_stop` goes out. The robot would keep moving after `robot.stop`. **A safety defect; it comes before any `goto` on the floor.**
2. **`robot.relax` and `robot.enable off` do not cancel it either.** `Disarm` zeroes the wheels and sends no stop (`s1d/src/tick.rs:458-463`).
3. **`s1.goto` while disarmed** sends its frame and marks the task, with no heartbeat. The effect is unknown.
4. **The lift rule's levels have no valid origin** (`s1d/src/tick.rs:248-259`). The test proves the mechanism.
5. **`robot.safeToRestart` looks only at the twist** (`s1d/src/ipc.rs:564-576`), so it says safe during `s1.wheels` or `s1.goto`.
6. **`safety.limp` means "disarmed" here** (`s1d/src/state.rs:356`): the console paints red and the panel alarms on a healthy disarmed robot (`mediad/webclient/index.html:1426-1428`, `duckctl/src/udp_pad/panel.rs:203-204`). No test covers it.
7. **`s1.armor` goes to `0x18`** (`s1d/src/ipc.rs:398`) where DJI's library goes to `0x09`, and the test does not prove the frame sent.
8. **No permission per method in `s1.*`**, including `s1.frame`, and the one-shot queue has no limit (`s1d/src/intents.rs:273`). It matters as soon as a remote route exists.
9. **`other =>`** (`s1d/src/ipc.rs:723`) lets a new contract call compile with nobody deciding.
10. **The deadman is in two places** with no test tying them: `s1d/src/main.rs:42-44` and `[safety]`.
11. **`/run` in the chroot is a plain directory** (`scripts/s1/build-rootfs.sh:40`), so a stale socket survives a reboot. Inferred; to confirm on the robot.
12. **`s1-infer` reads `sysconf` as `i64`** (`s1d/src/bin/s1-infer.rs:57`), wrong on 32 bits: the CPU-per-frame figure is not reliable. `duck-detect/src/bin/duck-bench.rs:87-89` has the same mistake and is the Pollen's.
13. **The refutation scripts use `sed -i ''`**, macOS only (`scripts/s1/e2e-refute.sh:19`, `scripts/s1/telemetry-refute.sh:19`), which blocks CI.
14. **`dji-oracle.py` needs DJI's library from outside the repository** (`scripts/s1/dji-oracle.py:8-9`); the hex stays frozen in `s1-duml/tests/dji_oracle.rs`.
15. **Follow differs from the captured app:** it repeats gimbal mode 2 at 8 Hz; `s1d` sends it once.
16. **`every_call` covers 73 of the contract's 79 variants**, so the route test does not see six. The Pollen's; to report, not to fix here.
17. **`s1.goto`'s timer does not use the clamped distance.** The frame clamps to ±5 m, the timer uses the raw `x` and `y` (`s1d/src/ipc.rs:340`, `s1d/src/ipc.rs:359-362`). Run on the fake body: `x: 100000` puts 5 m on the wire and, by that formula, keeps the task 200 000 s, about 55 h; `x: 1e308` panics in `Duration::from_secs_f64` after the frame is queued, so the connection closes with no answer, the frame stands and the task is not marked, while the wheel frames go on over it. The daemon stays up.
18. **Disarming leaves a non-zero gimbal rate as the last command.** The rate block is inside `if armed` (`s1d/src/tick.rs:562`), so no zero rate follows `robot.relax`. Run on the fake body: the last `0x04/0x0c` is 30 °/s. Shutdown takes the same path, by reading.
19. **The angle hold does not stand down for `s1.gimbal sleep` or `recenter`** (`s1d/src/tick.rs:591-607`). Run on the fake body: one `sleep`, then six `0x3f/0xb0` in 0.5 s. This may be why a fold would not hold; not measured.
20. **Arming overrides `s1.follow` while the slot keeps it** (`s1d/src/tick.rs:453-454`, `s1d/src/ipc.rs:292`). Run on the fake body: `follow chassis_lead`, then `robot.enable`: `0x04/0x4c` goes `02 00`, then `00 00`, while `0x3f/0xb0` stays pitch-only, so nothing commands the yaw.
21. **The control loop can wait on a client.** `state.sound.lock()` blocks the loop's thread on a shot (`s1d/src/tick.rs:660-662`), and the socket holds that lock across the kill, wait and spawn of `aplay` (`s1d/src/ipc.rs:651-654`, `s1d/src/sound.rs:125-149`). Read, not run. The heartbeat lives in that thread.
22. **`s1.frame` is unfiltered** (`s1d/src/ipc.rs:269-285`): it accepts `0x48/0x02`, the node reset.
23. **The judge of `s1-e2e`'s F6 assumes a turn the library contradicts.** It sends `goto 1 -0.5 -90` and wants the attitude yaw, positive clockwise, to rise by 90 (`s1d/src/bin/s1-e2e.rs:2461`). By DJI's library that command would turn anticlockwise, and the item would fail on a correct robot. Read, not measured.
24. **Builders nothing in `s1d` calls:** `gimbal_degree`, `gimbal_subscribe_position`, `dds::reset_node`, `dds::del_sub`. They are held to the oracle or to the SDK's layout. `reset_node` must stay uncalled: one reset left the bus dead until a power cycle.
