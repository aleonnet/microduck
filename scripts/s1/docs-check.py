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
PROSE = [
    r'11 frames +.+',
    r'echo "stage 11 stops [^"]*" \| tee "\$D/blocked\.txt"',
    r'"ON THE FLOOR[^"]*",',
    r'"the gimbal folds down[^"]*",',
    r'"the chassis runs [^"]*",',
    r'needs \(`docs/[\w./-]+`, item 1 of 2026-10-30 \(night\)\):',
    r'"(the floor\'s worst case must not trip|a disagreement of 0\.13 m/s must not trip the detector)"',
    r'"(the floor\'s worst case is not a lift|a disagreement of 0\.13 m/s is not a lift)"',
    # `infer`'s comment and message, which named a stage that fills nothing, and what they say now.
    'FRAMES="\\$\\{S1_FRAMES:\\-\\$OUT/frames\\}"\\ \\ \\ \\#\\ stage\\ 11\'s\\ PNG\\ lands\\ there;\\ any\\ directory\\ of\\ jpg/png\\ works',
    'FRAMES="\\$\\{S1_FRAMES:\\-\\$OUT/frames\\}"\\ \\ \\ \\#\\ nothing\\ fills\\ it\\ now;\\ any\\ directory\\ of\\ jpg/png\\ works,\\ such\\ as\\ `camera`\'s',
    '\\[\\ \\-n\\ "\\$\\(find\\ "\\$FRAMES"\\ \\-maxdepth\\ 1\\ \\\\\\(\\ \\-iname\\ \'\\*\\.png\'\\ \\-o\\ \\-iname\\ \'\\*\\.jpg\'\\ \\-o\\ \\-iname\\ \'\\*\\.jpeg\'\\ \\\\\\)\\ \\-print\\ \\-quit\\ 2>/dev/null\\)"\\ \\]\\ \\|\\|\\ \\{\\ say\\ "no\\ frames\\ in\\ \\$FRAMES\\ —\\ run\\ stage\\ 11\\ or\\ set\\ S1_FRAMES";\\ exit\\ 1;\\ \\}',
    '\\[\\ \\-n\\ "\\$\\(find\\ "\\$FRAMES"\\ \\-maxdepth\\ 1\\ \\\\\\(\\ \\-iname\\ \'\\*\\.png\'\\ \\-o\\ \\-iname\\ \'\\*\\.jpg\'\\ \\-o\\ \\-iname\\ \'\\*\\.jpeg\'\\ \\\\\\)\\ \\-print\\ \\-quit\\ 2>/dev/null\\)"\\ \\]\\ \\|\\|\\ \\{\\ say\\ "no\\ frames\\ in\\ \\$FRAMES\\ —\\ set\\ S1_FRAMES";\\ exit\\ 1;\\ \\}',
    # The refusals that named a plan's phase, and what they say now: these lines exactly.
    '"the\\ policy\\ is\\ not\\ driving:\\ this\\ body\\ has\\ no\\ skills\\ yet\\ \\(phase\\ 2\\)",',
    '"the\\ policy\\ is\\ not\\ driving:\\ this\\ body\\ has\\ no\\ skills\\ at\\ all",',
    'refused\\(id,\\ "no\\ voice\\ on\\ this\\ body\\ yet,\\ so\\ no\\ chorale\\ \\(phase\\ 2\\)"\\)',
    'refused\\(id,\\ "no\\ voice\\ on\\ this\\ body\\ yet,\\ so\\ no\\ chorale"\\)',
    '"no\\ kinematic\\ model\\ for\\ this\\ body\\ yet\\ \\(phase\\ 5\\ brings\\ the\\ URDF\\)",',
    '"no\\ kinematic\\ model\\ for\\ this\\ body\\ yet:\\ no\\ URDF\\ has\\ been\\ written",',
    'refused\\(id,\\ "this\\ body\\ keeps\\ no\\ vet\\ book\\ yet\\ \\(phase\\ 2\\)"\\)',
    'refused\\(id,\\ "this\\ body\\ keeps\\ no\\ vet\\ book,\\ not\\ even\\ an\\ empty\\ one"\\)',
    '"this\\ body\\ runs\\ no\\ policy\\ network\\ and\\ has\\ no\\ skills\\ yet\\ \\(phase\\ 2\\)",',
    '"this\\ body\\ runs\\ no\\ policy\\ network\\ and\\ has\\ no\\ skills\\ at\\ all",',
]
# A comment, by what the language says one is. Any other kind of file changing is logic.
COMMENT = {".rs": "//", ".sh": "#", ".py": "#", ".toml": "#", ".conf": "#"}
# The commits of the session the owner discarded, which the changelog must hold under that heading.
DISCARDED_COMMITS = "c0aab5c 9b40ff7 67b16f5 3db7e58 a196808 75b0c60 333f075 9bd3800 1832c99 27d4b6e".split()
# Cited as evidence nowhere but in the superseded copy.
DISCARDED_PATHS = ["poc2/floor", "poc2/up/", "poc2/refute"]
# Roots whose paths a page may name without a line number.
REPO_ROOTS = "s1d s1-duml scripts deploy mediad robotctl netpadd pad-map duck-ipc-proto robotd-params duckctl".split()


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
    dispatch = re.sub(r"//[^\n]*", "", ipc[ipc.index("pub fn dispatch(") :])
    named = set(re.findall(r"proto::Call::(\w+)", dispatch))
    refused = set()
    for arm in re.findall(r"((?:proto::Call::\w+(?:\(\w+\))?\s*\|?\s*)+)=>\s*\{?\s*refused\(", dispatch):
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
    s1 = set(re.findall(r'pub const [A-Z_0-9]+:\s*&str\s*=\s*"(s1\.[a-z_0-9]+)"', api))
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
                if t.split("/")[0] in REPO_ROOTS and "/" in t and ":" not in t and "*" not in t and "<" not in t:
                    if not (ROOT / t.rstrip("/")).exists() and not re.search(r"[{}]", t):
                        fails.append(f"stale path: {rel}:{n} names {t}: no such path")
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


def check_tests(fails):
    """Every function named in a `Test` column of the commands page exists in the S1's code."""
    code = run("git", "grep", "-h", "-o", "-E", r"fn [a-z0-9_]+", "--", "s1d", "s1-duml").stdout
    known = set(code.replace("fn ", "").split())
    column = None
    for n, line in enumerate(read(COMMANDS).split("\n"), 1):
        if not line.startswith("|"):
            column = None
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if "Test" in cells:
            column = cells.index("Test")
        elif column is not None and column < len(cells):
            for t in re.findall(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`", cells[column]):
                if t not in known:
                    fails.append(f"bad citation: {COMMANDS}:{n} names the test {t}: no such function")


def check_changelog_groups(fails):
    log = read(CHANGELOG)
    m = re.search(r"^### [^\n]*discarded by the owner[^\n]*\n.*?(?=^#{2,3} )", log, re.M | re.S)
    if not m:
        fails.append(f"commit missing: no group of the discarded session in {CHANGELOG}")
        return
    for h in DISCARDED_COMMITS:
        if f"`{h}`" not in m.group(0):
            fails.append(f"commit missing: {h} is not under the discarded session in {CHANGELOG}")


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
    flat = lambda t: re.sub(r"\s+", " ", t)
    record = flat(read(RECORD))
    for s in WITHDRAWN:
        if flat(s) in record:
            fails.append(f"withdrawn sentence: {s!r} is in {RECORD}")
    for rel in CHECKED:
        text = read(rel)
        for s in DISCARDED_PATHS:
            if s in text:
                fails.append(f"withdrawn sentence: {rel} cites {s}, the discarded session")
    found = run("git", "grep", "--untracked", "-l", "-F", "-e", "poc2/floor", "-e", "poc2/up/", "--", *S1_CODE,
                ":!scripts/s1/docs-check.py")
    for f in found.stdout.split():
        fails.append(f"withdrawn sentence: {f} cites the discarded session")


def comments_only(base, fails):
    diff = run("git", "diff", "-U0", base, "--", ".", ":!docs", ":!README.md", ":!CONTRIBUTING.md")
    if diff.returncode != 0:
        fails.append(f"logic line: git diff {base} failed: {diff.stderr.strip()}")
        return
    current = None
    for raw in diff.stdout.split("\n"):
        # A new file has `--- /dev/null`, a deleted one `+++ /dev/null`: the other line names it.
        if raw.startswith(("--- a/", "+++ b/")):
            current = raw[6:]
            continue
        if raw in ("--- /dev/null", "+++ /dev/null"):
            continue
        if not raw or raw[0] not in "+-":
            continue
        if current is None or current == "scripts/s1/docs-check.py":
            continue
        text = raw[1:].strip()
        if not text or any(re.fullmatch(p, text) for p in PROSE):
            continue
        comment = COMMENT.get(pathlib.Path(current).suffix)
        if comment is None or not text.startswith(comment):
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
        check_tests(fails)
        check_changelog_groups(fails)
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
