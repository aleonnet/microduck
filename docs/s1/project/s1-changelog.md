# S1: what changed, from the code and the history

The 78 commits of `main..27d4b6e`, by subject, with what each group left and what stands behind it. Commit messages are not taken as fact here: several call
proven what the owner later discarded. What is true of the code today is in [`docs/s1/design/s1-body.md`](../design/s1-body.md) and
[`docs/s1/robot/s1-commands.md`](../robot/s1-commands.md); the evidence is in [`s1-bringup.md`](s1-bringup.md). Nothing has been pushed.

`scripts/s1/docs-check.py` fails if a commit of that range is missing here or if a hash cited does not exist.

### Before the robot (2026-10-29/30, first night)

The protocol crate, the daemon, the scripts and the first cold reader's rounds. Code and tests; the wheels and gimbal had moved on 2026-10-29 from the lab's frames, before these.

`917dacb` · `5c73947` · `2a1a89f` · `c28bfe3` · `c47730c` · `88187a9` · `26a0e04` · `4d9ca3c`

### Stages 1 and 2 on the robot (2026-10-30, morning)

Baud, real LED command, backup with equal hashes. Proven on the robot.

`1ae2a52` · `52d2bf6`

### DJI's library as oracle, sound, the S1's own list

Builders held byte for byte to DJI's packer; `robot.sound`; `s1.*`. Code and tests.

`ba3a86f` · `da1872d` · `9b100cc` · `ea9b274` · `2e0e21e`

### Everything through the duck's mechanism, no robot

`s1-e2e fake`. Builds its own datagrams; not `duckctl`.

`cf3e344` · `b325606` · `885c1bc` · `56bb5a0` · `73d9e8f`

### Robot session (2026-10-30, afternoon and night)

Wheels in the air, gimbal, lights, sound, armour, infrared, button through `netpadd`, `robotctl`, `s1ctl`, with the owner as witness. The closest proof to the duck's path.

`bf8d1ac` · `1b9c7ee` · `8616bde` · `3849b7e`

### Stages 7 and 8, first attempts

The camera's sequence decoded after two reboots; speaker proven; a microphone conclusion withdrawn later.

`01bd80f` · `b8df6ea`

### Hand-offs and their corrections

Documents, with the bench's stages 7 and 11 stopping before any generic stream (`26fe0f5`). The hand-offs are now superseded.

`5ad758c` · `b535a8b` · `a7aa5be` · `26fe0f5`

### Telemetry (2026-10-30, night)

The stock's push and the gimbal's decoded; gimbal freed at arm; head relative to the body; two block orders; pick-up bit never set; lift rule asked for. Accepted by the owner.

`77a25fc` · `27d685e` · `91fb4e5` · `707f563` · `241c5f7` · `47b64e2` · `e84210d` · `ba06259` · `c53be46` · `fb95dc7` · `30834d6` · `7c2d444` · `4f4a582` · `408ff36` · `53b51a5` · `df40c2b`

### The camera (2026-10-31)

`s1-cam`: 27 steps replayed, 30.1 fps, to files. Proven on the robot.

`494fd67` · `ed080e1` · `e1307e5`

### Inference (2026-10-31)

`s1-infer`, 2342 ms, then NEON, 1741 ms. By hand. Its CPU figure is not reliable.

`000474f` · `0b09952` · `1ef2837` · `40bd0aa`

### USB and the Coral (2026-10-31)

No enumeration; "no 5 V" is an inference; parked.

`07113f7` · `40a7052` · `e35dba1` · `3301f22` · `8a98c12` · `e1ab58b` · `cb75063`

### The microphone (2026-10-31)

Two wrong readings, then the owner recognised the voice on the left channel. By hand; `s1d` has no route.

`6cab725` · `ec09d56` · `393a49e` · `7e940ef` · `6962e3e` · `3a7fe30` · `9633dfa`

### The duck's voice and ear

Synthesis played on the speaker; `micleft`; the petting model never fires here. By hand.

`1642642`

### **The floor session and its consequences (2026-10-31): sem prova válida, discarded by the owner**

Commit messages call these proven or fixed; none of that counts. The code they added exists and has tests: `robot.look` served, `s1.goto` owning the wheels, follow without the re-send, gimbal rate streamed, acks kept, mixer state, lift rule, `up` and `floor`. Thresholds, the control's name and the mixer file come from the session.

`c0aab5c` · `9b40ff7` · `67b16f5` · `3db7e58` · `a196808` · `75b0c60` · `333f075` · `9bd3800` · `1832c99` · `27d4b6e`

## After `27d4b6e`

This revision of the documents, on the branch as later commits: comments and help texts corrected, with no change of logic, and the S1's documents moved to `docs/s1/` by the owner's order, with the duck's documents back as they are on `main`. `git log 27d4b6e..` lists them. Then the Pollen's `main` (`b2b0d21`, 0.18.3, contract 47) merged in, without conflict, on the owner's order to keep up: six new contract calls, `os.*` and `telemetry.*`, which `s1d` does not name.

## History rewritten, 2026-11-01

On the owner's decision, the `Co-Authored-By` lines of nine commits were removed before any push, in a fresh clone, limited to `f746b2e^..s1-fase1`. For all 78 commits the tree, author and dates were compared old against new: 0 differences. Only these nine changed identity. Older documents cite the old hashes, including the [superseded copy of the record](2026-11-01-0032-s1-bringup-ate-57e01f2.md).

| Old | New |
|---|---|
| `f746b2e` | `9b40ff7` |
| `58a020b` | `67b16f5` |
| `2faf204` | `3db7e58` |
| `3c33456` | `a196808` |
| `f2e0f5a` | `75b0c60` |
| `c29adbc` | `333f075` |
| `b21c9ea` | `9bd3800` |
| `82938be` | `1832c99` |
| `57e01f2` | `27d4b6e` |

The old history is kept in the local branch `backup/s1-fase1-antes-dos-trailers` and in a bundle in the lab, `out/2026-11-01-historia/`, with the full table. Unchanged and cited by older text: `ec09d56`, `5ad758c`, `4d9ca3c`, `88187a9`.

## The front of 2026-10-30, abandoned

The work plan of 2026-10-30 was closed as abandoned on 2026-11-01 with its reason on record: phase 1 was not proven through the duck's path, the floor proof was discarded by the owner, and its eighth gate was red. The code stayed. Cold readers read parts of this branch on 2026-10-30: two rounds up to `88187a9`, both rejected; the first cut of `s1-e2e`, before `b325606`; a hand-off, before `26fe0f5`. The 48 commits after `26fe0f5` have had no such reading, and the rounds before it were not of the whole. That is the code front's to do.

### `rms1d`, etapas 1 e 2 (2026-11-02, branch `rms1d`)

Serviço novo, com `s1d` sem mudança: `rms1-proto`, `rms1d-params`, `rms1-control` e `rms1d`, no desenho do pato e com bytes da biblioteca. Um comando, `scripts/rms1d-sim`, sobe tudo no simulador. Testado pelo caminho do robô, sem janela em `scripts/test-rms1d-sim.sh` e pilotando o console no navegador. A prova no robô está em [`../robot/rms1d-roteiro-chassi-torre.md`](../robot/rms1d-roteiro-chassi-torre.md).

