status: accepted

# Handoff — overnight s1-fase1 — 2026-10-30T03:45:39Z

> Read this first. The state below was measured by `overnight-close.sh` at 2026-10-30T03:45:39Z.
> Nothing was pushed, bumped or released: those are yours to order.

## State (measured now)
| Repository | Branch | HEAD | Tree | Unpushed |
|---|---|---|---|---|
| microduck | s1-fase1 | 26a0e04 | clean | no upstream |

Gates: every declared gate FRESH on this tree (`close.sh --check`), evidence in
`.roadworthy/evidence.jsonl`.

## Where the real state lives
- `docs/decisions/2026-10-29-2334-overnight-s1-fase1.md` — the decision diary: phase ledger, decisions with sources, blockers
- `/Users/alessandro/.claude/plans/pasted-content-id-caf1-retomando-o-cuddly-bear.md` — the plan executed (master plan; `.diff-review-r1.md` / `-r2.md` beside it are the cold reader's two rounds on the night's diff)
- `/Users/alessandro/.claude/plans/2026-10-30-0330-s1-fase2-inteligencia.md` — the Phase 2 plan, born ready: pre-flight green (30 acceptance rows), two cold-review rounds in `.review.md` (round 2: 2 blockers, both measured and applied; ceiling reached — the `owner:` line is yours)
- `docs/project/s1-bringup.md` — the record: what is settled, the 12 bench stages, 12 artefacts with SHA (`scripts/s1/bench.sh verify-record docs/project/s1-bringup.md`)
- `~/Development/RoboMaster-S1/README.md` — the lab: where tonight's binaries, rootfs, ONNX Runtime and model are

## Blockers for the morning
- `1793327702058` · 2026-10-30T02:35:02Z · Dono: criar .roadworthy/overnight-rules (deny adb/setenforce/sudo) e .roadworthy/docs.json (decisions → ~/Development/RoboMaster-S1/diario) — o portão rite_gate não deixa o agente escrevê-los; à noite a disciplina é minha: nenhum adb/setenforce/sudo/push

## Bench (fill the result column on the real thing)
Before stage 1: `export S1_HOST=<ip>:5555` (root adb from the app's Lab), wheels OFF the ground from stage 4 on, every stage prints `artefato:` lines — paste them into `docs/project/s1-bringup.md`. Stages 3, 5, 11 and 12 decide the port; stage 2 is the backup.

| step | action | expected result | result |
|---|---|---|---|
| 0 | `cargo test -p s1-duml -p s1d` on the Mac | 33 passed (already measured tonight) | |
| 1 | `scripts/s1/bench.sh capture` — stock ALIVE; use the app for 70 s (camera, sound, hit each plate, LED colours, button, person) | `capture/*.txt`, two `.strace`, `top_vision.txt`; `stty -a` shows the baud → `export S1_BAUD=<n>` | |
| 2 | `scripts/s1/bench.sh backup` | `backup/bk/*.img` + `SHA256SUMS`; "SHA robot == Mac" | |
| 3 | `scripts/s1/bench.sh exec` (if `EXEC_IN_DATA_DENIED`, you run `setenforce 0` and re-run) | `CHROOT_OK`, `uname -a` 3.10.62, `GLIBC 2.36` | |
| 4 | `scripts/s1/bench.sh stop` | `hdvt_uav_stopped`, ttyS3 held by nobody | |
| 5 | `scripts/s1/bench.sh drive`, then on the Mac `duckctl udp-pad <ip>`; Start arms, sticks drive | wheels (in the air) and gimbal obey; `health.json` healthy; `monitor.log` shows battery/attitude/odom from DDS | |
| 6 | `scripts/s1/bench.sh events`; tap each plate, press the button, point an IR source | LEDs change on `robotctl flashlight`; `s1d.log` names plate and button; `event0.hex` non-empty | |
| 7 | `scripts/s1/bench.sh camera` | `camera/frame.png` visible | |
| 8 | `scripts/s1/bench.sh audio` (say something; drive during `aplay`) | `mic.wav` audible; 60 s stream without a gap; heartbeat keeps ticks | |
| 9 | `scripts/s1/bench.sh app` | notes only | |
| 10 | `scripts/s1/bench.sh restore` (reboot) | the app drives again | |
| 11 | `scripts/s1/bench.sh frames` | `fps_720p.txt`, `fps_360p.txt` with fps and CPU; `sample.png` | |
| 12 | `scripts/s1/bench.sh infer` (needs stage 11's `sample.png`) | `onboard.txt` with p50 ms/frame on the S1; `offboard.txt` on the Mac | |

Then: ratify the night's decisions in the diary; create `.roadworthy/overnight-rules` and `.roadworthy/docs.json` (owner-only); approve the Phase 2 plan in plan mode and ratify its three deviations from the master (QR instead of ArUco; `robot.event` instead of `robot.state.hits`; `robotctl` deferred to Phase 3).

## Scoreboard — what this front got wrong on the way
<!-- One line per class of failure the front exposed, each naming the defect number it came from.
     A class with no number is a story; a number with no class is a ticket. Both are needed, and
     an empty table means nobody looked. -->
| class | what it was | defect |
|---|---|---|
| dispatch not checked against the whole enum | 11 `robot.*` routes fell to METHOD_NOT_FOUND | diff r1 B1 |
| data path not followed end to end | `flashlight` on/off never wrote what `toggle` read | diff r1 B2 |
| scripts written without a local simulation | backup SHA formats, non-existent binary, glob guard, `--seconds`, `bs=1m`, pipefail on `health` | diff r1 B3/B4/W1–W4, r2 B1 |
| crate inventory not opened | `duck-bench` is the RKNN path; the S1 has no NPU | diff r1 B4 |
| consumer function not read whole | `btd`/`mediad` `permits` are exhaustive; a new `Call` touches four files | plan r1 B1 |
| gate text not tokenised before promising | diff gate with a missing quote | plan r1 B2 |
| formula not checked against the limit | yaw "clamp" that `atan2` cannot produce | plan r1 B3 |
| a reviewer's claim taken as fact | "btd does not build on the Mac" — it does (measured) | plan r2 B1 |
| transport attributed to a stage that did not measure it | frames for `s1-mind` "from stage 11" | plan r2 B2 |

## Prompts
- Bancada passou (3, 5, 11, 12 provados): "A bancada da Fase 1 passou: estágios <lista>. Cole as linhas `artefato:` no registro, feche a frente e abra a Fase 2 pelo plano `2026-10-30-0330-s1-fase2-inteligencia.md`."
- Bancada falhou: "A bancada da Fase 1 falhou no estágio <n>: <erro literal / o que o robô fez>. Diagnostique antes de propor."
- Só o plano: "Aprovo o plano da Fase 2 com os desvios (a) (b) (c) ratificados; abra a frente."
