#!/usr/bin/env python3
"""Decode an strace of `dji_hdvt_uav` (octal escapes included) into DUML frames: a histogram of
(cmd_set, cmd_id) written and read on the UART, with samples. What stage 1 of the bench produces
and what found the real LED command and the stock DDS subscription (2026-10-30).

Usage: scripts/s1/duml-strace.py <file.strace> [cmd ...]
"""
import re,sys,collections
def decode(s):
    out=bytearray(); i=0
    while i<len(s):
        c=s[i]
        if c=='\\':
            n=s[i+1]
            if n=='x': out.append(int(s[i+2:i+4],16)); i+=4; continue
            if n in '01234567':
                j=i+1; digs=''
                while j<len(s) and len(digs)<3 and s[j] in '01234567': digs+=s[j]; j+=1
                out.append(int(digs,8)&255); i=j; continue
            m={'n':10,'t':9,'r':13,'"':34,'\\':92,'a':7,'b':8,'f':12,'v':11}; out.append(m.get(n,ord(n))); i+=2; continue
        out.append(ord(c)); i+=1
    return bytes(out)
NAME={(0x3f,0x20):'wheel_speed',(0x3f,0x21):'chassis_speed',(0x3f,0x60):'heartbeat',(0x3f,0x19):'workmode',(0x3f,0xb0):'gimbal_angles',(0x3f,0xb2):'gimbal_recenter',(0x3f,0x33):'SYSTEM_LED',(0x3f,0x02):'ARMOR_HIT',(0x3f,0x10):'IR_HIT',(0x3f,0x51):'BLASTER',(0x48,0x08):'dds_push',(0x48,0x03):'dds_add_sub',(0x3f,0x55):'led_trail?',(0x04,0x69):'0x04/0x69',(0x0a,0x53):'0x0a/0x53'}
rx=re.compile(r'^(\d+)\s+(\S+) (\w+)\((\d+), "(.*?)"(\.\.\.)?, (\d+)\)\s*=\s*(-?\d+)')
path=sys.argv[1]; want=set(sys.argv[2:])
writes=collections.Counter(); reads=collections.Counter(); samples=collections.defaultdict(list); fds=collections.Counter(); trunc=0
for line in open(path,encoding='latin1'):
    m=rx.search(line)
    if not m: continue
    pid,ts,call,fd,payload,dots,n,rc=m.groups(); fd=int(fd)
    if dots: trunc+=1
    b=decode(payload); fds[(call,fd)]+=1
    if b[:1]==b'\x55' and len(b)>=11:
        key=(b[9],b[10]); (writes if call=='write' else reads)[(fd,key)]+=1
        name=NAME.get(key,'%02x/%02x'%key)
        if name in want and len(samples[name])<8: samples[name].append((ts, call, fd, b.hex()))
print("truncated payloads (-s 64):",trunc)
print("fds:",fds.most_common(6))
print("WRITES:",[(fd,NAME.get(k,'%02x/%02x'%k),n) for (fd,k),n in writes.most_common(16)])
print("READS :",[(fd,NAME.get(k,'%02x/%02x'%k),n) for (fd,k),n in reads.most_common(16)])
for name,rows in samples.items():
    print("--",name)
    for r in rows: print("  ",r)
