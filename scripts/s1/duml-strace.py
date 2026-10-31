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
       pitch/yaw extremes seen, to hold against the manual's limits; `gimbal_vs_attitude` pairs
       every gimbal push with the controller push nearest on their shared clock and prints how
       ground yaw − chassis-relative yaw sits against the attitude yaw, and the two pitch fields
       (offsets 0 and 2) against the attitude pitch — the stage `telemetry-free` report.
       scripts/s1/duml-strace.py --selftest   checks the pairing over frames built here.
"""
import re, struct, sys, collections, statistics

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

def encode(sender, receiver, seq, cmd_set, cmd_id, data, attri=0x00):
    """A whole DUML frame with both CRCs, version 1 — for the self-test's synthetic straces."""
    ln = 13 + len(data)
    head = bytes([0x55, ln & 0xff, 0x04 | (ln >> 8)])
    body = head + bytes([crc8(0x77, head), sender, receiver, seq & 0xff, seq >> 8, attri, cmd_set, cmd_id]) + bytes(data)
    c = crc16(0x3692, body)
    return body + bytes([c & 0xff, c >> 8])

STOCK_PUSH_PAYLOAD = 161   # after sub_mode and msg_id; the controller's boot-time subscription
AT_STOCK_TIME_MS = 0; AT_STOCK_ATTITUDE = 132   # `s1-duml/src/dds.rs::StockPush`

def pair_gimbal_with_attitude(per_stream, max_dt_ms=30):
    """[(t_ms, dt_ms, pitch0, pitch2, yaw4, yaw6, yaw8, att_yaw, att_pitch, att_roll)] — a gimbal
    push and the controller push nearest to it on their shared millisecond clock, read side only."""
    g = []; s = []
    for (call, fd), frs in per_stream.items():
        if call != 'read': continue
        for b in frs:
            d = b[11:-2]
            if (b[9], b[10]) == (0x04, 0x05) and len(d) >= 40:
                g.append((struct.unpack_from('<I', d, 12)[0],) + struct.unpack_from('<hhhhh', d, 0))
            elif (b[9], b[10]) == (0x48, 0x08) and len(d) == 2 + STOCK_PUSH_PAYLOAD:
                pl = d[2:]
                s.append((struct.unpack_from('<I', pl, AT_STOCK_TIME_MS)[0],) + struct.unpack_from('<fff', pl, AT_STOCK_ATTITUDE))
    s.sort(); keys = [x[0] for x in s]; rows = []
    import bisect
    for tms, p0, p2, y4, y6, y8 in g:
        i = bisect.bisect_left(keys, tms); cands = [j for j in (i - 1, i) if 0 <= j < len(s)]
        if not cands: continue
        j = min(cands, key=lambda j: abs(keys[j] - tms)); dt = tms - keys[j]
        if abs(dt) > max_dt_ms: continue
        rows.append((tms, dt, p0 / 10, p2 / 10, y4 / 10, y6 / 10, y8 / 10) + s[j][1:])
    return len(g), len(s), rows

def wrap(a): return (a + 180) % 360 - 180

def report_gimbal_vs_attitude(per_stream):
    ng, ns, rows = pair_gimbal_with_attitude(per_stream)
    out = [f"-- gimbal_vs_attitude: gimbal pushes {ng}, controller pushes {ns}, paired within 30 ms {len(rows)}"]
    if not rows: return out
    res = [wrap(r[4] - r[6] - r[7]) for r in rows]
    out.append("   ground yaw(@4) - relative yaw(@8) - attitude yaw: median %+.2f  min %+.2f  max %+.2f deg" % (statistics.median(res), min(res), max(res)))
    ap = [r[8] for r in rows]
    out.append("   attitude: yaw %.1f..%.1f  pitch %.2f..%.2f  roll %.2f..%.2f deg" % (min(r[7] for r in rows), max(r[7] for r in rows), min(ap), max(ap), min(r[9] for r in rows), max(r[9] for r in rows)))
    for name, idx in (("pitch@0", 2), ("pitch@2", 3)):
        v = [r[idx] for r in rows]; d = [a - b for a, b in zip(v, ap)]
        out.append("   %s: %.1f..%.1f deg; minus attitude pitch: median %+.2f range %+.2f..%+.2f" % (name, min(v), max(v), statistics.median(d), min(d), max(d)))
    out.append("   field @6: %.1f..%.1f" % (min(r[5] for r in rows), max(r[5] for r in rows)))
    return out

def selftest():
    """Two gimbal pushes and two controller pushes, 10 ms apart on the clock, with a 2.5° chassis
    yaw and the ground yaw = relative + chassis: the report must pair both and read a residual of
    zero; a pitch field put 7° from the attitude pitch must show as 7°."""
    def gimbal(tms, p0, p2, y4, y8):
        d = struct.pack('<hhhhh', p0, p2, y4, 0, y8) + bytes(2) + struct.pack('<I', tms) + bytes(8) + struct.pack('<ffff', 1, 0, 0, 0)  # 40 B: the real push's layout
        return encode(0x04, 0xf1, 1, 0x04, 0x05, d)
    def stock(tms, yaw, pitch):
        # Literal offsets, not the constants above: a report built and read with the same wrong
        # constant would agree with itself (the refutation script plants 120 and must see this fail).
        pl = bytearray(161); struct.pack_into('<I', pl, 0, tms); struct.pack_into('<fff', pl, 132, yaw, pitch, 0.0)
        return encode(0x03, 0x09, 1, 0x48, 0x08, bytes([0, 1]) + bytes(pl))
    frames_ = [stock(1000, 2.5, -1.0), gimbal(1010, 60, 0, 925, 900), stock(1020, 2.5, -1.0), gimbal(1030, 60, 0, -1775, -1800)]
    per = {('read', 16): frames_}
    text = "\n".join(report_gimbal_vs_attitude(per))
    want = ["paired within 30 ms 2", "median +0.00  min +0.00  max +0.00", "pitch@0: 6.0..6.0 deg; minus attitude pitch: median +7.00", "pitch@2: 0.0..0.0 deg; minus attitude pitch: median +1.00"]
    bad = [w for w in want if w not in text]
    # the strace text path too: the frames written as a read() line must decode to the same report
    import tempfile, os
    with tempfile.NamedTemporaryFile('w', suffix='.strace', delete=False) as f:
        for fr in frames_:
            f.write('1 00:00:00.000000 read(16, "%s", 256) = %d\n' % (''.join('\\x%02x' % b for b in fr), len(fr)))
        path = f.name
    per2, _ = frames(path); os.unlink(path)
    if "\n".join(report_gimbal_vs_attitude(per2)) != text: bad.append("strace text path differs from the direct one")
    if bad:
        print("duml-strace selftest FAILED:", bad, "\n" + text, file=sys.stderr); return 1
    print("duml-strace selftest: gimbal_vs_attitude pairs and reads as expected"); return 0

def main():
    if '--selftest' in sys.argv[1:]:
        sys.exit(selftest())
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
    if 'gimbal_vs_attitude' in want:
        for line in report_gimbal_vs_attitude(per_stream): print(line)

if __name__ == '__main__':
    main()
