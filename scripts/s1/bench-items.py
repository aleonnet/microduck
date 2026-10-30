#!/usr/bin/env python3
"""The body's items, one by one, through the S1's own list (`s1.*`) and the duck's shared routes — stages 5 and 6 of
`docs/project/s1-bringup.md` for everything beyond wheels-and-gimbal-by-angle.

Each item calls the real route; `s1.frame` with DJI's own bytes (`scripts/s1/dji-oracle.py`) only where
the list has no route, waits, and prints what the owner must have seen; the
daemon's answer and the owner's verdict go to the log. Run with the robot's socket forwarded:
`adb forward tcp:9000 localfilesystem:/data/debian/run/robotd.sock`, `s1d` and the chassis armed
by `bench-drive.py` or by `robot.enable` here. Wheels off the ground.

Usage: scripts/s1/bench-items.py <log file> [item ...]      (no items = all, in order)
"""
import json, os, socket, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
log = open(sys.argv[1], "a")
wanted = sys.argv[2:]
oracle = {}
for line in subprocess.run([os.path.join(HERE, "dji-oracle.py")], capture_output=True, text=True, check=True).stdout.splitlines():
    name, hex_ = line.split()
    oracle[name] = hex_

s = socket.create_connection(("127.0.0.1", 9000), timeout=5)
f = s.makefile("rw")
n = 0


def call(method, params):
    global n
    n += 1
    f.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": method, "params": params}) + "\n")
    f.flush()
    r = f.readline().strip()
    log.write(f"{time.strftime('%H:%M:%S')} {method} {json.dumps(params)} -> {r}\n")
    log.flush()
    return r


def frame(name):
    return call("s1.frame", {"hex": oracle[name]})


def witness(item, expect):
    print(f"\n[{item}] expected: {expect}")
    seen = input("  what happened? (enter = as expected) ").strip() or "as expected"
    log.write(f"{time.strftime('%H:%M:%S')} WITNESS {item}: {seen}\n")
    log.flush()


ITEMS = [
    # Each item is the real route of the S1's list; `frame(...)` (DJI's own bytes) stays only where
    # the list has no route yet. Expectations are what the owner must see, wheels in the air.
    ("F1 gimbal_lead", lambda: (call("s1.follow", {"mode": "gimbal_lead"}), time.sleep(0.5),
        [call("robot.head", {"neck_pitch": 0, "head_pitch": 0, "head_yaw": 0.8, "head_roll": 0}) or time.sleep(0.1) for _ in range(20)],
        time.sleep(2), [call("robot.head", {"neck_pitch": 0, "head_pitch": 0, "head_yaw": 0.0, "head_roll": 0}) or time.sleep(0.1) for _ in range(20)]),
        "gimbal yaws 45° and the chassis (wheels in the air) turns to follow it, then both come back"),
    ("F1 chassis_lead", lambda: (call("s1.follow", {"mode": "chassis_lead"}), time.sleep(0.5),
        [call("robot.move", {"vx": 0, "vy": 0, "vyaw": 0.6}) or time.sleep(0.1) for _ in range(20)], call("robot.stop", {})),
        "the chassis turns and the gimbal keeps pointing with the chassis (gimbal follows)"),
    ("F1 free", lambda: call("s1.follow", {"mode": "free"}), "nothing moves; back to independent chassis and gimbal"),
    ("F2 recenter", lambda: (frame("gimbal_degree_task7_cur_yaw300_pitchm100_acc120"), time.sleep(2), call("s1.gimbal", {"action": "recenter"})),
        "gimbal goes yaw +30° pitch −10°, then recentres"),
    ("F3 gimbal_speed", lambda: (call("s1.gimbal", {"action": "speed", "yaw": 30, "pitch": -15}), time.sleep(1), call("s1.gimbal", {"action": "stop"}), time.sleep(0.5), call("s1.gimbal", {"action": "recenter"})),
        "gimbal sweeps yaw right at 30°/s and pitch down at 15°/s for 1 s, stops, then recentres"),
    ("F4 gimbal_fpv", lambda: (call("s1.follow", {"mode": "free", "gimbal_fpv": True}), time.sleep(1), call("s1.follow", {"mode": "free"})), "no visible change (FPV then free); the answers are what counts"),
    ("F5 gimbal_sleep", lambda: (call("s1.gimbal", {"action": "sleep"}), time.sleep(3), call("s1.gimbal", {"action": "wake"}), time.sleep(2)), "gimbal goes limp and folds down, then wakes and levels"),
    ("F6 goto", lambda: (call("s1.goto", {"x": 1.0, "y": -0.5, "yaw": -90, "speed": 0.5, "yaw_speed": 30}), time.sleep(6)), "wheels run the pattern of a 1 m forward, 0.5 m right, 90° turn move and stop by themselves"),
    ("F7 velocity_frame", lambda: (call("s1.follow", {"mode": "free", "velocity_frame": "gimbal"}), time.sleep(0.5), call("s1.follow", {"mode": "free"})), "no visible change; answers only"),
    ("F9 blaster_led", lambda: (call("s1.lights", {"effect": "solid", "r": 255, "g": 255, "b": 255, "t1": 100, "t2": 100, "blaster": True}), time.sleep(2)), "the blaster's LED lights white"),
    ("F10 led_effects", lambda: [(call("s1.lights", dict(effect=e, r=r, g=g, b=b, t1=t1, t2=t2)), time.sleep(2.5)) for e, r, g, b, t1, t2 in (("solid", 255, 0, 0, 0, 0), ("breath", 0, 0, 255, 500, 500), ("flash", 0, 255, 0, 250, 250), ("marquee", 255, 255, 255, 100, 100), ("off", 0, 0, 0, 0, 0))],
        "every LED solid red, then breathing blue, then flashing green, then a white marquee, then off"),
    ("F11 armor_sensitivity", lambda: call("s1.armor", {"k": 1.0}), "no visible change; the answer says whether the armour host takes it"),
    ("F12 pwm", lambda: call("s1.pwm", {"duty": 500, "hz": 1000}), "a scope on a PWM port would show 1 kHz at 50 %; answers only here"),
    ("F15 chassis_workmode", lambda: (frame("chassis_workmode_2"), time.sleep(0.5), frame("chassis_workmode_1")), "no visible change; answers only"),
    ("F16 wheels", lambda: ([call("s1.wheels", {"rpm": [100, 100, 100, 100]}) or time.sleep(0.1) for _ in range(20)], call("robot.stop", {})), "all four wheels forward at 100 rpm for 2 s, then stop"),
    ("F17 blaster_ir", lambda: call("s1.blaster", {"times": 1}), "the blaster's infrared fires once (no gel: tank out); a second S1 or the app would register the hit"),
    ("F18 telemetry", lambda: print("   ", call("s1.telemetry", {})[:400]), "battery %, attitude and gimbal angle plausible (not 10^32); pushes > 0"),
]

call("robot.health", {})
for item, act, expect in ITEMS:
    if wanted and not any(item.startswith(w) for w in wanted):
        continue
    act()
    time.sleep(0.5)
    witness(item, expect)
call("s1.mode", {"mode": "free"})
print("done; log:", sys.argv[1])
