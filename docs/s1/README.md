# The RoboMaster S1

The owner, 2026-11-01: **"pato e s1 não se misturam. Eu nem pato tenho. Usam a mesma estrutura do diretorio apenas".**
So the S1's documents live in this tree, with the same folders the duck's documents use, and no page outside it mentions the S1.
`scripts/s1/docs-check.py` fails with `mixed` if that changes. The S1's code is in `s1d/`, `s1-duml/`, `scripts/s1/` and `deploy/s1/`.

**Where it stands:** nothing is proven yet through `duckctl udp-pad` or the browser. Start at [`project/s1-bringup.md`](project/s1-bringup.md#state-today-2026-11-01-code-at-27d4b6e-no-robot-touched-in-this-revision).

One page owns a mechanism, and the others link to it. When two pages disagree, the one that does not own it is the bug; when the code and a page disagree, the page is the bug.

| Page | What it owns |
|---|---|
| [`design/s1-body.md`](design/s1-body.md) | How the S1 runs the software today: the loop, telemetry, what is outside `s1d`, what is reused, every part by component, and the target architecture, proposed. |
| [`robot/s1-commands.md`](robot/s1-commands.md) | Every command: the frame, the test, the proof on the robot, what reaches it, what is missing. Open defects. The base for wiring commands to the console and the pad. |
| [`robot/s1-bench.md`](robot/s1-bench.md) | Running it from a Mac, from a cold robot, with what each step has really proved. |
| [`project/s1-bringup.md`](project/s1-bringup.md) | The dated record: what was measured, with its artefacts. Every part by stage. |
| [`project/s1-changelog.md`](project/s1-changelog.md) | The commits by subject, what stands behind each group, the hash table of the rewritten history. |
| [`project/2026-11-01-1000-conferencia-das-paginas.md`](project/2026-11-01-1000-conferencia-das-paginas.md) | What was checked on each page, against what, and what stays unmeasured. |

Kept because revising means a new file and the old one stays; superseded, not current:

| Page | |
|---|---|
| [`project/2026-11-01-0032-s1-bringup-ate-57e01f2.md`](project/2026-11-01-0032-s1-bringup-ate-57e01f2.md) | The record as it was, including the floor session the owner discarded. |
| [`plans/2026-10-30-0045-handoff-overnight-s1-fase1.md`](plans/2026-10-30-0045-handoff-overnight-s1-fase1.md) | Hand-off of the first night. |
| [`plans/2026-10-30-1905-handoff-s1-fase1-bateria2.md`](plans/2026-10-30-1905-handoff-s1-fase1-bateria2.md) | Hand-off of the second battery; its name and hour are wrong, as the record says. |

The raw matter, captures and partition images, is in the lab, `~/Development/RoboMaster-S1/`, outside this repository.
