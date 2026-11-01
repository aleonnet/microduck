#!/usr/bin/env python3
"""The S1's documents against the code. What a machine can hold them to; it cannot tell whether a
sentence is right, only that what it cites exists.

    scripts/s1/docs-check.py                    every check below but --comments-only
    scripts/s1/docs-check.py --record           only the record's withdrawn sentences
    scripts/s1/docs-check.py --comments-only B  the diff from B to the working tree touches, in code,
                                                only comments and the few help texts listed here

Checks: every call `s1d` names, `robot.*` and `s1.*`, is in docs/s1/robot/s1-commands.md in the right
section, and every call there is in the code (`command missing`, `command unknown`); a citation
`path:line` resolves to a line that exists (`bad citation`); a hash cited exists in git (`bad hash`);
every commit of the S1's range is in the changelog (`commit missing`); every part H1..H72 is in both
tables (`piece missing`); the record holds none of the withdrawn sentences (`withdrawn sentence`);
the duck's documents are as on `main` and do not mention the S1 (`mixed`: "pato e s1 não se
misturam", the owner, 2026-11-01; bring `main` up to date when the Pollen's commits come in); a path
under docs/ named by a page or by the S1's code exists (`stale path`).

Exit 0 when clean, 1 with one line per failure.
"""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
COMMANDS = "docs/s1/robot/s1-commands.md"
RECORD = "docs/s1/project/s1-bringup.md"
BODY = "docs/s1/design/s1-body.md"
CHANGELOG = "docs/s1/project/s1-changelog.md"
BENCH = "docs/s1/robot/s1-bench.md"
CHECKED = ["docs/s1/README.md", COMMANDS, RECORD, BODY, CHANGELOG, BENCH]
# The duck's documents, which hold nothing of the S1's, and the S1's code, whose comments cite pages.
DUCK_DOCS = ["docs", "README.md", "CONTRIBUTING.md", "AGENTS.md", "CLAUDE.md", ":!docs/s1"]
S1_CODE = ["s1d", "s1-duml", "scripts/s1", "deploy/s1", "Cargo.toml", ".cargo"]
# The S1's commits before this revision, which the changelog must hold.
RANGE = "main..27d4b6e"

PIECES = """H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11 H12 H13 H14 H15 H16 H20 H21 H22 H23 H24 H30 H31 H32 H33 H34
H35 H36 H37 H38 H39 H40 H41 H42 H43 H44 H45 H46 H50 H51 H52 H60 H61 H62 H63 H70 H71 H72""".split()

# Artefacts are cited relative to this lab directory; checked when the lab is on this machine.
LAB = pathlib.Path.home() / "Development/RoboMaster-S1/out/2026-10-30-poc2"
# Files of DJI's firmware that the record cites by line. They are not in this tree.
OUTSIDE = {"script_framework.py", "rm_module.py", "rm_ctrl.py", "rm_define.py", "command_subjects.hpp"}

# What the revision of 2026-11-01 took out of the record: false, withdrawn or from the session the
# owner discarded. A sentence coming back is a regression. (`docs/s1/project/s1-changelog.md`)
WITHDRAWN = [
    "nothing is inferred from behaviour alone",
    "mic: device `hw:0,0` right channel, no signal yet",
    "every item pre-tested on the Mac",
    "the 38 items",
    "stays on the robot for the",
    "telemetry is closed",
    "not yet in code",
    "and holds no voice",
    "one codec register apart",
    "proposed to the owner, not yet done",
    "OTG Y cable (ordered)",
    "Until `s1-cam` exists",
    "`s1.mode`",
    "The owner's ruling",
    "the six hit detectors",
    "press and long\n  press are the two events",
    "'Speaker Volume'`)",
    "a flag file in `/data` chooses",
    "## Floor session",
    "## Audit ledger",
    "## What closes phase 1",
    "## The fixes of the audit ledger",
    "### Runbook",
    "### Reuse map",
    "## State at the end of 2026-10-31",
    "poc2/floor/",
    "poc2/up/",
    "until a floor session",
    " his ",
    " he ",
]

# Help and prompt texts that the comments-only check accepts besides comments.
PROSE = ['11 frames ', 'echo "stage 11 stops', '"ON THE FLOOR', '"the gimbal folds down', 'needs (`docs/']


def run(*args):
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True)


def read(rel):
    return (ROOT / rel).read_text(encoding="utf-8")


def contract():
    """(method name by variant, method names by variant named in s1d's dispatch, refused variants)."""
    proto = read("duck-ipc-proto/src/lib.rs")
    consts = dict(re.findall(r'pub const ([A-Z_0-9]+):\s*&str\s*=\s*"([^"]+)"', proto))
    start = proto.index("pub fn method(&self)")
    body = proto[start : proto.index("\n    }\n", start)]
    names = {v: consts[c] for v, c in re.findall(r"Call::(\w+)(?:\(_\))? => method::(\w+)", body)}
    ipc = read("s1d/src/ipc.rs")
    dispatch = ipc[ipc.index("pub fn dispatch(") :]
    named = set(re.findall(r"proto::Call::(\w+)", dispatch))
    refused = set()
    for arm in re.findall(r"((?:proto::Call::\w+(?:\(_\))?\s*\|?\s*)+)=>\s*\{?\s*refused\(", dispatch):
        refused |= set(re.findall(r"proto::Call::(\w+)", arm))
    return names, named, refused


def section_calls(doc, heading):
    """The backticked first cell of each row of the table under a `## ` heading."""
    m = re.search(r"^## " + re.escape(heading) + r".*?(?=^## |\Z)", doc, re.M | re.S)
    if not m:
        return None
    return re.findall(r"^\| `([^`]+)` \|", m.group(0), re.M)


def check_commands(fails):
    doc = read(COMMANDS)
    names, named, refused = contract()
    api = read("s1d/src/api.rs")
    s1 = set(re.findall(r'pub const [A-Z]+: &str = "(s1\.[a-z]+)"', api))
    want = {
        "`robot.*` the body serves": {names[v] for v in named if v in names} - {names[v] for v in refused},
        "`robot.*` the body refuses with a reason": {names[v] for v in refused},
        "`s1.*`, the S1's own list": s1,
        "Contract calls `s1d` does not name": {names[v] for v in names if v not in named},
    }
    if named - set(names):
        fails.append(f"command unknown: s1d names {sorted(named - set(names))}, not in the contract")
    for heading, calls in want.items():
        rows = section_calls(doc, heading)
        if rows is None:
            fails.append(f"command missing: no section {heading!r} in {COMMANDS}")
            continue
        for c in sorted(calls - set(rows)):
            fails.append(f"command missing: {c} is in the code and not under {heading!r} in {COMMANDS}")
        for c in sorted(set(rows) - calls):
            fails.append(f"command unknown: {c} is under {heading!r} in {COMMANDS} and not so in the code")


def tree():
    return [p for p in run("git", "ls-files", "--cached", "--others", "--exclude-standard").stdout.split("\n") if p]


def check_citations(fails):
    files = tree()
    by_name = {}
    for f in files:
        by_name.setdefault(f.rsplit("/", 1)[-1], []).append(f)
    token = re.compile(r"`([^`\s]*)`")
    for rel in CHECKED:
        for n, line in enumerate(read(rel).split("\n"), 1):
            last = None
            for t in token.findall(line):
                # `path:12`, `path:12-34`, `path:12,34` and, after one, a bare `:56`.
                m = re.fullmatch(r"([\w./+-]*):(\d+)(?:-(\d+))?((?:,\d+)*)", t)
                if not m or not (m.group(1) or last):
                    continue
                path = m.group(1) or last
                base = path.rsplit("/", 1)[-1]
                if "." not in base or re.fullmatch(r"[\d.]+", path):
                    continue
                if path in files:
                    target = ROOT / path
                elif "/" not in path and len(by_name.get(path, [])) >= 1:
                    target = ROOT / by_name[path][0]
                elif base in OUTSIDE:
                    continue
                elif not LAB.is_dir() and "/" in path:
                    continue
                elif (LAB / path).is_file():
                    target = LAB / path
                else:
                    fails.append(f"bad citation: {rel}:{n} cites {t}: no such file")
                    continue
                last = path
                size = len(target.read_text(encoding="utf-8", errors="replace").splitlines())
                lines = [int(m.group(2))] + ([int(m.group(3))] if m.group(3) else [])
                lines += [int(x) for x in m.group(4).split(",") if x]
                for ln in lines:
                    if ln < 1 or ln > size:
                        fails.append(f"bad citation: {rel}:{n} cites {t}: the file has {size} lines")


def check_hashes(fails):
    for rel in CHECKED:
        for n, line in enumerate(read(rel).split("\n"), 1):
            for t in re.findall(r"`([0-9a-f]{7,40})`", line):
                if not re.search(r"[a-f]", t) or not re.search(r"[0-9]", t):
                    continue
                if run("git", "cat-file", "-e", t + "^{commit}").returncode != 0:
                    fails.append(f"bad hash: {rel}:{n} cites {t}: not in git")
    log = read(CHANGELOG)
    got = run("git", "log", "--format=%h", "--abbrev=7", RANGE)
    if got.returncode != 0:
        fails.append(f"commit missing: git log {RANGE} failed: {got.stderr.strip()}")
    for h in got.stdout.split():
        if f"`{h}`" not in log:
            fails.append(f"commit missing: {h} is not in {CHANGELOG}")


def check_pieces(fails):
    record = read(RECORD)
    for p in PIECES:
        if not re.search(r"^\| " + p + r" \|", record, re.M):
            fails.append(f"piece missing: {p} has no row in the record's table, {RECORD}")
    body = read(BODY)
    for p in PIECES:
        if not re.search(r"\b" + p + r"\b", body):
            fails.append(f"piece missing: {p} is not in the tables and lists of {BODY}")
    if len(re.findall(r"^\| H\d+[ ,]", body, re.M)) < 22:
        fails.append(f"piece missing: the part by component table of {BODY} has under 22 rows")


def check_mixed(fails):
    diff = run("git", "diff", "--name-only", "main", "--", *DUCK_DOCS)
    if diff.returncode != 0:
        fails.append(f"mixed: git diff main failed: {diff.stderr.strip()}")
    for f in diff.stdout.split():
        fails.append(f"mixed: {f} differs from main; the S1's documents live in docs/s1")
    found = run("git", "grep", "--untracked", "-l", "-i", "-E", "s1d|s1-duml|robomaster|docs/s1", "--", *DUCK_DOCS)
    for f in found.stdout.split():
        fails.append(f"mixed: {f} mentions the S1; the S1's documents live in docs/s1")


def check_paths(fails):
    pattern = re.compile(r"\bdocs/[\w./-]+\.md\b")
    for rel in CHECKED:
        for n, line in enumerate(read(rel).split("\n"), 1):
            for path in pattern.findall(line):
                if not (ROOT / path).is_file():
                    fails.append(f"stale path: {rel}:{n} names {path}: no such file")
    found = run("git", "grep", "--untracked", "-n", "-o", "-E", r"docs/[A-Za-z0-9_./-]+\.md", "--", *S1_CODE)
    for hit in found.stdout.split("\n"):
        if hit:
            rel, n, path = hit.split(":", 2)
            if not (ROOT / path).is_file():
                fails.append(f"stale path: {rel}:{n} names {path}: no such file")


def check_record(fails):
    record = read(RECORD)
    for s in WITHDRAWN:
        if s in record:
            fails.append(f"withdrawn sentence: {s!r} is in {RECORD}")


def comments_only(base, fails):
    paths = ["s1d", "s1-duml", "scripts/s1", "deploy/s1", "Cargo.toml"]
    diff = run("git", "diff", "-U0", base, "--", *paths)
    if diff.returncode != 0:
        fails.append(f"logic line: git diff {base} failed: {diff.stderr.strip()}")
        return
    current = None
    for raw in diff.stdout.split("\n"):
        if raw.startswith("+++ "):
            current = raw[6:] if raw.startswith("+++ b/") else None
            continue
        if raw.startswith("--- "):
            if current is None and raw.startswith("--- a/"):
                current = raw[6:]
            continue
        if not raw or raw[0] not in "+-":
            continue
        if current is None or current == "scripts/s1/docs-check.py":
            continue
        text = raw[1:].strip()
        if not text or any(text.startswith(p) for p in PROSE):
            continue
        comment = "//" if current.endswith(".rs") else "#"
        if not text.startswith(comment):
            fails.append(f"logic line: {current}: {raw[:100]}")


def main():
    args = sys.argv[1:]
    fails = []
    if args[:1] == ["--comments-only"]:
        if len(args) != 2:
            sys.exit("usage: docs-check.py --comments-only <base>")
        comments_only(args[1], fails)
    elif args == ["--record"]:
        check_record(fails)
    elif args:
        sys.exit(__doc__)
    else:
        check_commands(fails)
        check_citations(fails)
        check_hashes(fails)
        check_pieces(fails)
        check_record(fails)
        check_mixed(fails)
        check_paths(fails)
    for f in fails:
        print(f)
    if fails:
        print(f"docs-check: {len(fails)} failures", file=sys.stderr)
        return 1
    print("docs-check: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
