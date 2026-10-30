#!/usr/bin/env python3
"""Stage 5 without a gamepad: drive `s1d` over its JSON-RPC socket from the Mac, through
`adb forward tcp:9000 localfilesystem:/data/debian/run/robotd.sock`. Arms, drives forward, turns,
strafes, moves the gimbal, disarms; every answer goes to the log given as the first argument.

Usage: scripts/s1/bench-drive.py <log file>
"""
import socket, json, time, sys
log = open(sys.argv[1], "w")
s = socket.create_connection(("127.0.0.1", 9000), timeout=5); f = s.makefile("rw"); n = 0
rtts = []
def call(method, params, quiet=False):
    global n; n += 1
    t0 = time.perf_counter()
    f.write(json.dumps({"jsonrpc":"2.0","id":n,"method":method,"params":params}) + "\n"); f.flush()
    r = f.readline().strip(); rtts.append((time.perf_counter() - t0) * 1e3)
    log.write(f"{time.strftime('%H:%M:%S')} {method} {json.dumps(params)} -> {r} rtt_ms={rtts[-1]:.1f}\n")
    if not quiet: print(method, json.dumps(params), "->", r[:160])
    return r
def hold(method, params, secs, hz=10):
    t_end = time.time() + secs
    while time.time() < t_end:
        call(method, params, quiet=True); time.sleep(1/hz)
call("robot.health", {})
call("robot.enable", {"on": True, "toggle": False})
time.sleep(1.0); call("robot.health", {})
print("--- forward 0.2 m/s, 3 s"); hold("robot.move", {"vx":0.2,"vy":0.0,"vyaw":0.0}, 3)
call("robot.stop", {}); time.sleep(1)
print("--- turn left 0.6 rad/s, 2 s"); hold("robot.move", {"vx":0.0,"vy":0.0,"vyaw":0.6}, 2)
call("robot.stop", {}); time.sleep(1)
print("--- strafe left 0.2 m/s, 2 s"); hold("robot.move", {"vx":0.0,"vy":0.2,"vyaw":0.0}, 2)
call("robot.stop", {}); time.sleep(1)
print("--- gimbal yaw +0.5 rad, pitch +0.3 rad, 2 s each, then centre")
hold("robot.head", {"neck_pitch":0,"head_pitch":0.0,"head_yaw":0.5,"head_roll":0}, 2)
hold("robot.head", {"neck_pitch":0,"head_pitch":0.3,"head_yaw":-0.5,"head_roll":0}, 2)
hold("robot.head", {"neck_pitch":0,"head_pitch":0.0,"head_yaw":0.0,"head_roll":0}, 1)
call("robot.enable", {"on": False, "toggle": False})
call("robot.health", {})
rtts.sort()
summary = f"socket round trip over {len(rtts)} calls: min {rtts[0]:.1f} ms · p50 {rtts[len(rtts)//2]:.1f} ms · max {rtts[-1]:.1f} ms (Mac → adb forward → chroot socket → s1d → back)"
print(summary); log.write(summary + "\n")
