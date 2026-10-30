#!/usr/bin/env python3
"""Decode an strace of a stock DJI process into DUML frames: a histogram of (cmd_set, cmd_id)
written and read on each fd, with samples of the commands asked for. What stage 1 of the bench
produces, and what found the real LED command, the stock DDS subscription (2026-10-30) and the
telemetry the controller pushes on its own (2026-10-30 (night)).

Frames are reassembled per fd and direction before decoding: the UART driver hands reads over in
pieces (a 4-byte header, then the rest), so a line-by-line decode sees whole frames only on the
write side. Each frame is held to both CRCs — DJI's CRC-8 (reflected polynomial 0x8C, seed 0x77)
and CRC-16 (reflected 0x8408, seed 0x3692), the tables `s1-duml/src/crc.rs` carries, generated
here from the polynomials (checked equal to the lab's tables, 2026-10-30 (night)).

Usage: scripts/s1/duml-strace.py <file.strace> [cmd ...]
       cmd names as in NAME below (e.g. dds_add_sub gimbal_push); `gimbal_push` also prints the
       pitch/yaw extremes seen, to hold against the manual's limits.
"""
import re, sys, collections

def _table(poly, width):
    out = []
    for i in range(256):
        c = i
        for _ in range(8):
            c = (c >> 1) ^ poly if c & 1 else c >> 1
        out.append(c & ((1 << width) - 1))
    return out

CRC8_TABLE = _table(0x8C, 8)
CRC16_TABLE = _table(0x8408, 16)

def crc8(seed, data):
    c = seed
    for d in data:
        c = CRC8_TABLE[c ^ d]
    return c

def crc16(seed, data):
    c = seed
    for d in data:
        c = (c >> 8) ^ CRC16_TABLE[(c ^ d) & 0xff]
    return c

def decode(s):
    out = bytearray(); i = 0
    while i < len(s):
        c = s[i]
        if c == '\\':
            n = s[i + 1]
            if n == 'x': out.append(int(s[i + 2:i + 4], 16)); i += 4; continue
            if n in '01234567':
                j = i + 1; digs = ''
                while j < len(s) and len(digs) < 3 and s[j] in '01234567': digs += s[j]; j += 1
                out.append(int(digs, 8) & 255); i = j; continue
            m = {'n': 10, 't': 9, 'r': 13, '"': 34, '\\': 92, 'a': 7, 'b': 8, 'f': 12, 'v': 11}; out.append(m.get(n, ord(n))); i += 2; continue
        out.append(ord(c)); i += 1
    return bytes(out)

NAME = {(0x3f, 0x20): 'wheel_speed', (0x3f, 0x21): 'chassis_speed', (0x3f, 0x60): 'heartbeat', (0x3f, 0x19): 'workmode',
        (0x3f, 0xb0): 'gimbal_angles', (0x3f, 0xb2): 'gimbal_recenter', (0x3f, 0x33): 'SYSTEM_LED', (0x3f, 0x32): 'system_led',
        (0x3f, 0x02): 'ARMOR_HIT', (0x3f, 0x10): 'IR_HIT', (0x3f, 0x51): 'BLASTER', (0x48, 0x08): 'dds_push',
        (0x48, 0x03): 'dds_add_sub', (0x48, 0x01): 'dds_add_node', (0x48, 0x02): 'dds_reset_node', (0x48, 0x04): 'dds_del_sub',
        (0x04, 0x05): 'gimbal_push', (0x3f, 0x55): 'gun_led', (0x04, 0x69): '0x04/0x69', (0x0a, 0x53): '0x0a/0x53'}

rx = re.compile(r'^(\d+)\s+(\S+) (read|write)\((\d+), "(.*?)"(\.\.\.)?, (\d+)\)\s*=\s*(-?\d+)')

def frames(path):
    """(call, fd, first timestamp of the stream) -> list of whole, CRC-valid frames, in order."""
    streams = collections.OrderedDict(); first_ts = {}; trunc = 0
    for line in open(path, encoding='latin1'):
        m = rx.search(line)
        if not m: continue
        pid, ts, call, fd, payload, dots, n, rc = m.groups()
        if dots: trunc += 1
        key = (call, int(fd))
        streams.setdefault(key, bytearray()).extend(decode(payload)); first_ts.setdefault(key, ts)
    out = collections.OrderedDict()
    for key, buf in streams.items():
        found = []; i = 0
        while i + 4 <= len(buf):
            if buf[i] != 0x55 or crc8(0x77, bytes(buf[i:i + 3])) != buf[i + 3]: i += 1; continue
            ln = buf[i + 1] | ((buf[i + 2] & 3) << 8)
            if ln < 13 or i + ln > len(buf): i += 1; continue
            fr = bytes(buf[i:i + ln])
            if crc16(0x3692, fr[:-2]) != (fr[-2] | (fr[-1] << 8)): i += 1; continue
            found.append(fr); i += ln
        out[key] = found
    return out, trunc

def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(2)
    path = sys.argv[1]; want = set(sys.argv[2:])
    per_stream, trunc = frames(path)
    print("truncated payloads (strace -s):", trunc)
    writes = collections.Counter(); reads = collections.Counter(); samples = collections.defaultdict(list)
    ext = None
    for (call, fd), frs in per_stream.items():
        for b in frs:
            key = (b[9], b[10]); (writes if call == 'write' else reads)[(fd, key)] += 1
            name = NAME.get(key, '%02x/%02x' % key)
            if name in want and len(samples[name]) < 8: samples[name].append((call, fd, b.hex()))
            if key == (0x04, 0x05) and len(b) >= 11 + 10 + 2:
                p = int.from_bytes(b[11:13], 'little', signed=True); y = int.from_bytes(b[15:17], 'little', signed=True)
                ext = (p, p, y, y) if ext is None else (min(ext[0], p), max(ext[1], p), min(ext[2], y), max(ext[3], y))
    print("frames per stream:", {f"{c}:{fd}": len(v) for (c, fd), v in per_stream.items() if v})
    print("WRITES:", [(fd, NAME.get(k, '%02x/%02x' % k), n) for (fd, k), n in writes.most_common(16)])
    print("READS :", [(fd, NAME.get(k, '%02x/%02x' % k), n) for (fd, k), n in reads.most_common(16)])
    for name, rows in samples.items():
        print("--", name)
        for r in rows: print("  ", r)
    if 'gimbal_push' in want:
        if ext is None:
            print("-- gimbal_push: none seen")
        else:
            print(f"-- gimbal_push extremes: pitch {ext[0] / 10:.1f}..{ext[1] / 10:.1f} deg (manual -20..+35)  yaw {ext[2] / 10:.1f}..{ext[3] / 10:.1f} deg (manual +-250)")

if __name__ == '__main__':
    main()
