#!/usr/bin/env python3
"""The body's items, one by one, through `s1d`'s bench routes — stages 5 and 6 of
`docs/project/s1-bringup.md` for everything beyond wheels-and-gimbal-by-angle.

Each item sends the frame DJI's own packer produces (`scripts/s1/dji-oracle.py`) via `s1.frame`,
or sets the follow mode via `s1.mode`, waits, and prints what the owner must have seen; the
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
    ("F1 gimbal_lead", lambda: (call("s1.mode", {"mode": "gimbal_lead"}), time.sleep(0.5),
        [call("robot.head", {"neck_pitch": 0, "head_pitch": 0, "head_yaw": 0.8, "head_roll": 0}) or time.sleep(0.1) for _ in range(20)],
        time.sleep(2), [call("robot.head", {"neck_pitch": 0, "head_pitch": 0, "head_yaw": 0.0, "head_roll": 0}) or time.sleep(0.1) for _ in range(20)]),
        "gimbal yaws 45° and the chassis (wheels in the air) turns to follow it, then both come back"),
    ("F1 chassis_lead", lambda: (call("s1.mode", {"mode": "chassis_lead"}), time.sleep(0.5),
        [call("robot.move", {"vx": 0, "vy": 0, "vyaw": 0.6}) or time.sleep(0.1) for _ in range(20)], call("robot.stop", {})),
        "the chassis turns and the gimbal keeps pointing with the chassis (gimbal follows)"),
    ("F1 free", lambda: call("s1.mode", {"mode": "free"}), "nothing moves; back to independent chassis and gimbal"),
    ("F2 recenter", lambda: (frame("gimbal_degree_task7_cur_yaw300_pitchm100_acc120"), time.sleep(2), frame("gimbal_recenter_task7_yaw100_pitch100")),
        "gimbal goes yaw +30° pitch −10°, then recentres"),
    ("F3 gimbal_speed", lambda: ([frame("gimbal_speed_yaw300_pitchm150_ctrl_dc") or time.sleep(0.1) for _ in range(10)], time.sleep(0.5), frame("gimbal_recenter_task7_yaw100_pitch100")),
        "gimbal sweeps yaw right at 30°/s and pitch down at 15°/s for 1 s, then recentres"),
    ("F4 gimbal_workmode", lambda: (frame("gimbal_workmode_1"), time.sleep(1), frame("gimbal_workmode_0")), "no visible change (FPV then free); the answers are what counts"),
    ("F5 gimbal_suspend", lambda: (frame("gimbal_suspend"), time.sleep(3), frame("gimbal_resume"), time.sleep(2)), "gimbal goes limp and folds down, then wakes and levels"),
    ("F6 position", lambda: (frame("chassis_position_task7_x100_ym50_yaw900_v50_w300"), time.sleep(6)), "wheels run the pattern of a 1 m forward, 0.5 m right, 90° turn move and stop by themselves"),
    ("F7 stick_overlay", lambda: (frame("chassis_stick_overlay_2"), time.sleep(0.5), frame("chassis_stick_overlay_0")), "no visible change; answers only"),
    ("F9 gun_led", lambda: (frame("gun_led_ctrl1_mode0"), time.sleep(2)), "the blaster's LED lights white"),
    ("F10 led_effects", lambda: [(frame(f"led_{n}"), time.sleep(2.5)) for n in ("solid_red", "breath_blue", "flash_green", "marquee_white", "off")],
        "every LED solid red, then breathing blue, then flashing green, then a white marquee, then off"),
    ("F11 armor_sensitivity", lambda: frame("armor_sensitivity_k1_to_hdvt"), "no visible change; the answer says whether the controller takes it"),
    ("F12 pwm", lambda: (frame("chassis_pwm_freq_all_1000"), frame("chassis_pwm_value_all_500")), "a scope on a PWM port would show 1 kHz at 50 %; answers only here"),
    ("F15 chassis_workmode", lambda: (frame("chassis_workmode_2"), time.sleep(0.5), frame("chassis_workmode_1")), "no visible change; answers only"),
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
