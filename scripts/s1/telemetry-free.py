#!/usr/bin/env python3
"""The gimbal freed from the chassis, measured through s1d's own list — stage `telemetry-free`.

With the stock stopped and `s1d` armed (chassis work mode 1, gimbal work mode 0), the head is swept
left and right by `robot.head` while `s1.telemetry` is sampled; then the head is held at zero for a
window in which the owner tilts the robot by hand. The samples decide three things the record
needs (`docs/s1/project/s1-bringup.md`, item 1 of 2026-10-30 (night)):

  body_still     the chassis did not follow the gimbal: the attitude yaw stayed put and the wheels
                 reported no speed while the head moved (wheels off the ground: a follow attempt
                 shows as rpm, never as motion);
  yaw_relation   ground yaw − chassis-relative yaw = attitude yaw, the reading of the `0x04/0x05`
                 push the layout was given on 2026-10-30, now with the body still;
  pitch_frame    whether the pitch the push carries at offset 0 moves with the chassis when the
                 chassis tilts (chassis-relative) or holds to the ground (ground frame) —
                 undecided when nobody tilted the robot.

Usage:
  telemetry-free.py run (--tcp HOST:PORT | --socket PATH) --out samples.ndjson
                        [--sweep S] [--hold S] [--rate HZ] [--yaw DEG] [--pitch DEG]
  telemetry-free.py judge samples.ndjson
  telemetry-free.py --selftest

`run` ends with `judge` over what it wrote. Exit 2 when the daemon cannot be reached.
"""
import argparse
import json
import math
import socket
import statistics
import sys
import time


class Rpc:
    """One NDJSON JSON-RPC connection to s1d (the shape `s1ctl` uses: a line out, a line back)."""

    def __init__(self, tcp=None, unix=None):
        if tcp:
            host, port = tcp.rsplit(":", 1)
            self.sock = socket.create_connection((host, int(port)), timeout=5)
        else:
            self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.sock.settimeout(5)
            self.sock.connect(unix)
        self.reader = self.sock.makefile("r", encoding="utf-8")
        self.next_id = 1

    def call(self, method, params=None):
        rid = self.next_id
        self.next_id += 1
        req = {"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}}
        self.sock.sendall((json.dumps(req) + "\n").encode())
        while True:
            line = self.reader.readline()
            if not line:
                raise ConnectionError("daemon closed the connection")
            msg = json.loads(line)
            if msg.get("id") == rid:  # notifications (s1.event) carry no id and are skipped
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result")


def head_params(yaw_deg, pitch_up_deg):
    """The duck's head: positive yaw looks LEFT, positive pitch noses DOWN (radians). DJI's gimbal
    is the other way round on both axes (`s1d::tick::gimbal_ddeg`), so a +yaw here is a −yaw on
    the wire and in the push."""
    return {
        "neck_pitch": 0.0,
        "head_pitch": -math.radians(pitch_up_deg),
        "head_yaw": math.radians(yaw_deg),
        "head_roll": 0.0,
    }


def run(args):
    try:
        rpc = Rpc(tcp=args.tcp, unix=args.socket)
    except OSError as e:
        print(f"cannot reach {args.tcp or args.socket}: {e}", file=sys.stderr)
        return 2
    out = open(args.out, "w", encoding="utf-8")
    t0 = time.monotonic()

    def sample(phase, yaw, pitch):
        tel = rpc.call("s1.telemetry")
        row = {"t": round(time.monotonic() - t0, 3), "phase": phase, "cmd_yaw_deg": yaw,
               "cmd_pitch_deg": pitch, "telemetry": tel}
        out.write(json.dumps(row) + "\n")
        out.flush()

    period = 1.0 / args.rate
    try:
        try:
            rpc.call("robot.enable", {"on": True, "toggle": False})
        except (ConnectionError, OSError, RuntimeError) as e:
            # An adb forward accepts the connection whether or not the daemon is behind it; the
            # daemon's absence shows here, on the first call (2026-10-30: s1d had died at start).
            print(f"no daemon behind {args.tcp or args.socket}: {e}", file=sys.stderr)
            return 2
        # The arm sequence (work mode, gimbal free mode, first heartbeat) goes out in the next tick.
        time.sleep(1.0)
        # Four legs: centre, left, right, centre — each a quarter of the sweep; pitch moves on the
        # two outer legs so the raw push (strace alongside) sees both axes change.
        legs = [(0.0, 0.0), (args.yaw, args.pitch), (-args.yaw, -args.pitch / 2), (0.0, 0.0)]
        leg_len = args.sweep / len(legs)
        for yaw, pitch in legs:
            rpc.call("robot.head", head_params(yaw, pitch))
            end = time.monotonic() + leg_len
            while time.monotonic() < end:
                sample("sweep", yaw, pitch)
                time.sleep(period)
        rpc.call("robot.head", head_params(0.0, 0.0))
        end = time.monotonic() + args.hold
        while time.monotonic() < end:
            sample("hold", 0.0, 0.0)
            time.sleep(period)
    finally:
        try:
            rpc.call("robot.enable", {"on": False, "toggle": False})
        except (OSError, RuntimeError, ConnectionError) as e:
            print(f"disarm failed: {e}", file=sys.stderr)
        out.close()
    return judge_file(args.out)


def wrap(deg):
    return (deg + 180.0) % 360.0 - 180.0


def span(values):
    """Largest angular separation in a set of degrees, wrap-safe for a set spread under 180°."""
    if not values:
        return 0.0
    ref = values[0]
    rel = [wrap(v - ref) for v in values]
    return max(rel) - min(rel)


def corr(a, b):
    if len(a) < 2:
        return float("nan")
    ma, mb = statistics.mean(a), statistics.mean(b)
    num = sum((x - ma) * (y - mb) for x, y in zip(a, b))
    den = math.sqrt(sum((x - ma) ** 2 for x in a) * sum((y - mb) ** 2 for y in b))
    return num / den if den else float("nan")


# Tolerances. The gimbal push and the controller push are 20 ms apart at most (same clock); at
# rest the two captures of 2026-10-30 put ground − relative − attitude at 1.0° and −0.2°.
BODY_YAW_TOL_DEG = 3.0
BODY_RPM_TOL = 20
RELATION_TOL_DEG = 3.0
HEAD_TOL_DEG = 5.0
TILT_MIN_DEG = 5.0


def judge(rows):
    """The verdict lines for a list of sample rows. Pure: the self-test feeds it synthetic rows."""
    lines = []
    with_push = [r for r in rows if r["telemetry"] and r["telemetry"].get("gimbal_deg") is not None
                 and r["telemetry"].get("attitude_deg") is not None]
    lines.append(f"samples {len(rows)}, with gimbal and attitude pushes {len(with_push)}")
    if len(with_push) < 10:
        lines.append("VERDICT impossible: fewer than 10 samples carry both pushes")
        return lines
    sweep = [r for r in with_push if r["phase"] == "sweep"]
    hold = [r for r in with_push if r["phase"] == "hold"]

    # body_still: attitude yaw and wheel rpm while the head moved
    att_yaw = [r["telemetry"]["attitude_deg"][0] for r in sweep]
    rpm = [max(abs(v) for v in r["telemetry"]["wheels_rpm"]) for r in sweep
           if r["telemetry"].get("wheels_rpm") is not None]
    yaw_span = span(att_yaw)
    if not sweep:
        lines.append("body_still undecided: no sweep window")
    elif not rpm:
        # s1d publishes no wheel speeds when the ESC block fails its plausibility (2026-10-30
        # 22:00: 10 675 rpm against a ±1000 motor); with the wheels off the ground the attitude
        # yaw cannot stand in for them, and the owner's eyes on the wheels are the verdict.
        lines.append(f"body_still undecided: no wheel speeds in the samples (ESC block not published); "
                     f"attitude yaw span {yaw_span:.1f} deg — the wheels are judged by eye")
    else:
        rpm_max = max(rpm)
        still = yaw_span <= BODY_YAW_TOL_DEG and rpm_max <= BODY_RPM_TOL
        lines.append(f"body_still {'yes' if still else 'NO'}: attitude yaw span {yaw_span:.1f} deg "
                     f"(tol {BODY_YAW_TOL_DEG}), max |rpm| {rpm_max} (tol {BODY_RPM_TOL})")

    # head_obeyed: at the end of each commanded leg the chassis-relative yaw is the command, in
    # DJI's sign (the duck's +yaw is DJI's −yaw)
    legs = {}
    for r in sweep:
        legs.setdefault(r["cmd_yaw_deg"], []).append(r)
    errs = []
    for cmd, rs in legs.items():
        if cmd == 0.0:
            continue
        last = rs[-1]["telemetry"].get("gimbal_yaw_vs_chassis_deg")
        if last is None:
            continue
        errs.append((cmd, last, abs(wrap(last - (-cmd)))))
    if errs:
        worst = max(e[2] for e in errs)
        detail = ", ".join(f"cmd {c:+.0f} -> relative {m:+.1f}" for c, m, _ in errs)
        lines.append(f"head_obeyed {'yes' if worst <= HEAD_TOL_DEG else 'NO'}: {detail} "
                     f"(worst {worst:.1f} deg, tol {HEAD_TOL_DEG})")
    else:
        lines.append("head_obeyed undecided: no chassis-relative yaw in the samples")

    # yaw_relation over the whole run
    resid = [wrap(r["telemetry"]["gimbal_deg"][0] - r["telemetry"]["gimbal_yaw_vs_chassis_deg"]
                  - r["telemetry"]["attitude_deg"][0])
             for r in with_push if r["telemetry"].get("gimbal_yaw_vs_chassis_deg") is not None]
    if resid:
        med = statistics.median(resid)
        spread = max(resid) - min(resid)
        lines.append(f"yaw_relation {'yes' if spread <= RELATION_TOL_DEG else 'NO'}: ground − relative − "
                     f"attitude = median {med:+.2f} deg, spread {spread:.2f} (tol {RELATION_TOL_DEG})")

    # pitch_frame from the hold window: did the push's pitch (offset 0) move with the chassis?
    att_pitch = [r["telemetry"]["attitude_deg"][1] for r in hold]
    g_pitch = [r["telemetry"]["gimbal_deg"][1] for r in hold]
    if len(hold) >= 10:
        tilt = max(att_pitch) - min(att_pitch)
        if tilt < TILT_MIN_DEG:
            lines.append(f"pitch_frame undecided: chassis pitch moved {tilt:.1f} deg in the hold "
                         f"window (needs {TILT_MIN_DEG})")
        else:
            moved = max(g_pitch) - min(g_pitch)
            c = corr(g_pitch, att_pitch)
            if moved >= 0.6 * tilt and abs(c) >= 0.8:
                verdict = "chassis-relative (moved with the chassis)"
            elif moved <= 0.3 * tilt:
                verdict = "ground frame (held while the chassis tilted)"
            else:
                verdict = "unclear"
            lines.append(f"pitch_frame {verdict}: chassis tilted {tilt:.1f} deg, push pitch moved "
                         f"{moved:.1f} deg, corr {c:+.2f}")
    else:
        lines.append("pitch_frame undecided: no hold window")
    return lines


def judge_file(path):
    rows = [json.loads(line) for line in open(path, encoding="utf-8") if line.strip()]
    for line in judge(rows):
        print(line)
    return 0


def _rows(phase, n, cmd_yaw, att_yaw, rel_yaw, rpm=0, att_pitch=0.0, g_pitch=0.0, offset=1.0):
    """Synthetic samples: ground yaw = relative + attitude + offset, the shape the robot gave.
    `rpm=None` is a daemon that published no wheel speeds."""
    out = []
    for i in range(n):
        ay = att_yaw(i) if callable(att_yaw) else att_yaw
        ry = rel_yaw(i) if callable(rel_yaw) else rel_yaw
        ap = att_pitch(i) if callable(att_pitch) else att_pitch
        gp = g_pitch(i) if callable(g_pitch) else g_pitch
        tel = {"gimbal_deg": [wrap(ry + ay + offset), gp], "gimbal_yaw_vs_chassis_deg": ry,
               "attitude_deg": [ay, ap, 0.0]}
        if rpm is not None:
            tel["wheels_rpm"] = [rpm, -rpm, rpm, -rpm]
        out.append({"t": i / 10, "phase": phase, "cmd_yaw_deg": cmd_yaw, "cmd_pitch_deg": 0.0,
                    "telemetry": tel})
    return out


def selftest():
    fails = []

    def expect(name, lines, *needles):
        text = "\n".join(lines)
        for n in needles:
            if n not in text:
                fails.append(f"{name}: missing {n!r} in:\n{text}")

    # A: body still, head obeyed, relation holds, owner tilted 15° and the push pitch held → ground
    rows = (_rows("sweep", 20, 0.0, 10.0, 0.0) + _rows("sweep", 20, 60.0, 10.0, -60.0)
            + _rows("sweep", 20, -60.0, 10.0, 60.0) + _rows("sweep", 20, 0.0, 10.0, 0.0)
            + _rows("hold", 30, 0.0, 10.0, 0.0, att_pitch=lambda i: 15.0 * (i % 2), g_pitch=0.5))
    expect("A", judge(rows), "body_still yes", "head_obeyed yes", "yaw_relation yes",
           "pitch_frame ground frame")
    # B: the body followed (attitude yaw walked 30°, wheels turned) and the pitch moved with the tilt
    rows = (_rows("sweep", 40, 60.0, lambda i: 10.0 + 0.75 * i, -60.0, rpm=120)
            + _rows("hold", 30, 0.0, 40.0, 0.0, att_pitch=lambda i: 15.0 * (i % 2),
                    g_pitch=lambda i: -15.0 * (i % 2)))
    expect("B", judge(rows), "body_still NO", "pitch_frame chassis-relative")
    # C: the relation breaks (ground yaw drifts from relative + attitude) and nobody tilted
    rows = (_rows("sweep", 40, 60.0, 10.0, -60.0, offset=0.0)
            + [dict(r, telemetry=dict(r["telemetry"], gimbal_deg=[r["telemetry"]["gimbal_deg"][0] + 8.0, 0.0]))
               for r in _rows("sweep", 20, 0.0, 10.0, 0.0, offset=0.0)]
            + _rows("hold", 30, 0.0, 10.0, 0.0, att_pitch=0.3))
    expect("C", judge(rows), "yaw_relation NO", "pitch_frame undecided")
    # D: too few pushes
    expect("D", judge(_rows("sweep", 5, 0.0, 0.0, 0.0)), "VERDICT impossible")
    # E: no wheel speeds published (the ESC block failed its plausibility) → the body is not
    # judged by numbers it does not have
    expect("E", judge(_rows("sweep", 40, 60.0, 10.0, -60.0, rpm=None)),
           "body_still undecided: no wheel speeds")
    if fails:
        print("\n".join(fails), file=sys.stderr)
        return 1
    print("telemetry-free selftest: 5 cases judged as expected")
    return 0


def main():
    if "--selftest" in sys.argv[1:]:
        return selftest()
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    where = r.add_mutually_exclusive_group(required=True)
    where.add_argument("--tcp")
    where.add_argument("--socket")
    r.add_argument("--out", required=True)
    r.add_argument("--sweep", type=float, default=30.0, help="seconds of head sweep (four legs)")
    r.add_argument("--hold", type=float, default=20.0, help="seconds held at zero for the tilt")
    r.add_argument("--rate", type=float, default=10.0, help="samples per second")
    r.add_argument("--yaw", type=float, default=60.0, help="sweep amplitude, degrees")
    r.add_argument("--pitch", type=float, default=15.0, help="pitch on the outer legs, degrees up")
    j = sub.add_parser("judge")
    j.add_argument("file")
    args = p.parse_args()
    if args.cmd == "run":
        return run(args)
    return judge_file(args.file)


if __name__ == "__main__":
    sys.exit(main())
