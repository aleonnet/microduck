#!/bin/sh
# O caminho inteiro, sem janela: `rms1d-roteiro`, socket, `rms1d`, quadros, ponte, corpo simulado. Os mesmos programas do
# robô, com o corpo por rede. A verdade do simulador confere o que o robô anda; os quadros se comparam com o corpo falso.
set -eu
REPO="$(cd "$(dirname "$0")/.." && pwd)"
LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
PYTHON="${PYTHON:-python3}"
T="$(mktemp -d /tmp/rms1t.XXXXXX)"
WS=18766
TCP=19000
QUADROS=19100
PIDS=""
encerrar() {
    for p in $PIDS; do kill "$p" 2>/dev/null || true; done
}
trap encerrar EXIT
falha() {
    echo "REPROVOU: $*"
    for f in ponte corpo rms1d; do
        if [ -f "$T/$f.log" ]; then
            echo "--- $f.log"
            tail -15 "$T/$f.log"
        fi
    done
    exit 1
}

cd "$REPO"
cargo build --quiet -p rms1d -p robotctl
RMS1D=target/debug/rms1d
ROTEIRO=target/debug/rms1d-roteiro
ROBOTCTL=target/debug/robotctl

"$PYTHON" "$LAB/tools/simulador/s1_ponte.py" --ws $WS --tcp $TCP --quadros $QUADROS > "$T/ponte.log" 2>&1 &
PIDS="$PIDS $!"
n=0
until grep -q "quadros em" "$T/ponte.log"; do
    n=$((n + 1)); [ "$n" -lt 50 ] || falha "a ponte não subiu"
    sleep 0.1
done
node "$LAB/tools/simulador/corpo_sem_navegador.mjs" "ws://127.0.0.1:$WS" > "$T/corpo.log" 2>&1 &
PIDS="$PIDS $!"
n=0
until grep -q "ponte ligada" "$T/corpo.log"; do
    n=$((n + 1)); [ "$n" -lt 150 ] || falha "o corpo simulado não ligou à ponte"
    sleep 0.1
done

# Fala com o corpo simulado pela porta de quem olha, nunca pelo caminho do robô.
verdade() {
    "$PYTHON" -I -c '
import json, socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])))
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": sys.argv[2]}) + "\n").encode())
f = s.makefile()
while True:
    v = json.loads(f.readline())
    if v.get("id") == 1:
        print(json.dumps(v["result"]))
        break
' "$TCP" "$1"
}
campo() { "$PYTHON" -I -c 'import json,sys; v=json.loads(sys.argv[1]); print(eval(sys.argv[2],{"v":v}))' "$1" "$2"; }

subir() { # subir <nome> <tipo>
    sock="$T/$1.sock"
    printf '[robo]\nnome = "teste"\n[corpo]\ntipo = "%s"\nendereco = "127.0.0.1:%s"\ngravar = "%s"\n' "$2" "$QUADROS" "$T/$1.quadros" > "$T/$1.toml"
    "$RMS1D" --config "$T/$1.toml" --socket "$sock" >> "$T/rms1d.log" 2>&1 &
    RMS1D_PID=$!
    PIDS="$PIDS $RMS1D_PID"
    n=0
    until [ -S "$sock" ]; do
        n=$((n + 1)); [ "$n" -lt 50 ] || falha "rms1d $1 não subiu"
        sleep 0.1
    done
}
descer() { kill "$RMS1D_PID" 2>/dev/null || true; wait "$RMS1D_PID" 2>/dev/null || true; }

echo "--- robotctl health pelo rms1d, com o corpo simulado"
subir sim rede
sleep 1.5
"$ROBOTCTL" --robot-socket "$T/sim.sock" --config-socket "$T/sim.sock" --socket "$T/sim.sock" health > "$T/saude.txt" 2>&1 || { cat "$T/saude.txt"; falha "robotctl health"; }
grep -q "robot     healthy" "$T/saude.txt" || { cat "$T/saude.txt"; falha "health não disse healthy"; }
echo "PASSOU robotctl health: $(grep 'robot     ' "$T/saude.txt")"

A0="$(verdade sim.verdade)"
[ "$(campo "$A0" 'v["armado"]')" = False ] || falha "o corpo armou sem pedido"
echo "PASSOU ligar não arma o corpo"

echo "--- o roteiro, pelo socket, contra o corpo simulado"
"$ROTEIRO" --socket "$T/sim.sock" || falha "roteiro contra o simulador"

Q="$(verdade sim.quadros)"
[ "$(campo "$Q" 'v["desconhecidos"]')" = 0 ] || falha "quadros desconhecidos: $Q"
[ "$(campo "$Q" 'v["descartados"]')" = 0 ] || falha "bytes descartados: $Q"
[ "$(campo "$Q" 'v["total"] > 100')" = True ] || falha "poucos quadros: $Q"
echo "PASSOU todos os quadros entendidos: $(campo "$Q" 'v["total"]')"
descer
sleep 1
A="$(verdade sim.verdade)"
[ "$(campo "$A" 'v["armado"]')" = False ] || falha "sem rms1d o corpo segue armado"
echo "PASSOU sem rms1d o corpo desarma"

echo "--- o mesmo roteiro manda os mesmos quadros em outro corpo"
subir falso falso
"$ROTEIRO" --socket "$T/falso.sock" > /dev/null || falha "roteiro contra o corpo falso"
descer
"$ROTEIRO" --comparar "$T/sim.quadros" "$T/falso.quadros" || falha "os quadros diferem entre os corpos"

echo "--- a verdade do simulador: o robô anda para onde o cliente pediu"
subir verdade rede
sleep 0.5
A0="$(verdade sim.verdade)"
"$ROTEIRO" --socket "$T/verdade.sock" --so-andar --velocidade 0.5 || falha "andar"
sleep 0.3
A1="$(verdade sim.verdade)"
FRENTE="$("$PYTHON" -I -c '
import json,math,sys
a,b=json.loads(sys.argv[1]),json.loads(sys.argv[2])
dx,dy=b["pos"][0]-a["pos"][0],b["pos"][1]-a["pos"][1]
y=a["yaw"]
print(round(math.cos(y)*dx+math.sin(y)*dy,3), round(-math.sin(y)*dx+math.cos(y)*dy,3))' "$A0" "$A1")"
"$PYTHON" -I -c '
import sys
f,l=map(float,sys.argv[1].split())
sys.exit(0 if 0.3<f<0.8 and abs(l)<0.05 else 1)' "$FRENTE" || falha "0,5 m/s por 1 s andou $FRENTE (frente, lado)"
echo "PASSOU andou para a frente: $FRENTE m (frente, lado)"
RPM="$(campo "$A1" 'max(abs(r) for r in v["rpm"])')"
[ "$RPM" -lt 15 ] || falha "ficou com rodas girando: $RPM rpm"
echo "PASSOU parou"
descer

echo "RMS1D SIM OK"
