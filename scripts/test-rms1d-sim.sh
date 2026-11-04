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

# A ponte precisa de `websockets`: por `uv`, na versão fixada, e nunca pelo Python do terminal. `$PYTHON` só roda biblioteca padrão.
VERSAO="$(sed -n 's/^websockets==//p' "$LAB/tools/requirements.txt")"
uv run --no-project --python 3.11 --with "websockets==$VERSAO" python "$LAB/tools/simulador/s1_ponte.py" --ws $WS --tcp $TCP --quadros $QUADROS > "$T/ponte.log" 2>&1 &
PIDS="$PIDS $!"
n=0
until grep -q "quadros em" "$T/ponte.log"; do
    n=$((n + 1)); [ "$n" -lt 600 ] || falha "a ponte não subiu"
    sleep 0.1
done
S1_MICRODUCK="$REPO" node "$LAB/tools/simulador/corpo_sem_navegador.mjs" "ws://127.0.0.1:$WS" > "$T/corpo.log" 2>&1 &
PIDS="$PIDS $!"
n=0
until grep -q "ponte ligada" "$T/corpo.log"; do
    n=$((n + 1)); [ "$n" -lt 150 ] || falha "o corpo simulado não ligou à ponte"
    sleep 0.1
done

# Fala com o corpo simulado pela porta de quem olha, nunca pelo caminho do robô.
# A telemetria que o rms1d decodificou, pelo socket dele.
tele() {
    "$PYTHON" -I -c '
import json, socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.sendall(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.telemetry\"}\n")
f = s.makefile()
while True:
    v = json.loads(f.readline())
    if v.get("id") == 1:
        print(json.dumps(v["result"]))
        break
' "$1"
}
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
"$ROTEIRO" --socket "$T/sim.sock" > "$T/roteiro.txt" 2>&1 || { cat "$T/roteiro.txt"; falha "roteiro contra o simulador"; }
cat "$T/roteiro.txt"
# O corpo simulado devolve a telemetria como o robô: "sem telemetria" aqui é defeito, e não corpo falso.
grep -q "^PASSOU a telemetria chega, com todos os dados" "$T/roteiro.txt" || falha "o roteiro não recebeu a telemetria do simulador"
echo "PASSOU a telemetria real chegou pelo caminho do robô"

Q="$(verdade sim.quadros)"
[ "$(campo "$Q" 'v["desconhecidos"]')" = 0 ] || falha "quadros desconhecidos: $Q"
[ "$(campo "$Q" 'v["descartados"]')" = 0 ] || falha "bytes descartados: $Q"
[ "$(campo "$Q" 'v["invalidos"]')" = 0 ] || falha "quadros inválidos: $Q"
[ "$(campo "$Q" 'v["avisos"]')" = 0 ] || falha "quadros com aviso: $Q"
[ "$(campo "$Q" 'v["total"] > 100')" = True ] || falha "poucos quadros: $Q"
echo "PASSOU todos os quadros entendidos: $(campo "$Q" 'v["total"]')"
# A sensibilidade das placas sai uma vez, com o primeiro pacote, às seis placas, no nível padrão: picos 160, 180 e 200
# (a0 00 b4 00 c8 00). O corpo aceita as seis (sem aviso, conferido acima).
[ "$(campo "$Q" '[(c["total"], c["ultimo"][26:]) for c in v["comandos"] if c["comando"] == "0x3f/0x07"]')" = "[(6, 'a000b400c800')]" ] || falha "a sensibilidade não chegou às seis placas, uma vez: $Q"
echo "PASSOU a sensibilidade chegou às seis placas, uma vez"
descer
sleep 1
A="$(verdade sim.verdade)"
[ "$(campo "$A" 'v["armado"]')" = False ] || falha "sem rms1d o corpo segue armado"
echo "PASSOU sem rms1d o corpo desarma"

echo "--- a verdade do simulador: cada sentido, nos três modos, pelo socket"
# pilotar <socket> <skill> <vx> <vy> <vyaw>: segura o pedido por 1,5 s a 10 Hz, como o console, e desarma.
pilotar() {
    "$PYTHON" -I -c '
import json, os, socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
f = s.makefile()
def chamar(m, p, i=[0]):
    i[0] += 1
    s.sendall((json.dumps({"jsonrpc": "2.0", "id": i[0], "method": m, "params": p}) + "\n").encode())
    while True:
        v = json.loads(f.readline())
        if v.get("id") == i[0]:
            return v
chamar("robot.enable", {"on": True})
time.sleep(0.1)
chamar("robot.do", {"skill": sys.argv[2]})
time.sleep(0.2)
vx, vy, vyaw = map(float, sys.argv[3:6])
if os.environ.get("HEAD_YAW"):
    chamar("robot.head", {"neck_pitch": 0, "head_pitch": 0, "head_yaw": float(os.environ["HEAD_YAW"]), "head_roll": 0})
fim = time.time() + 1.5
while time.time() < fim:
    s.sendall((json.dumps({"jsonrpc": "2.0", "method": "robot.move", "params": {"vx": vx, "vy": vy, "vyaw": vyaw}}) + "\n").encode())
    time.sleep(0.1)
chamar("robot.relax", {})
' "$@"
}
# mede <nome> <skill> <vx> <vy> <vyaw> <condição em frente, lado, giro>
mede() {
    subir sentido rede
    sleep 0.5
    M0="$(verdade sim.verdade)"
    pilotar "$T/sentido.sock" "$2" "$3" "$4" "$5" || falha "$1: pilotar"
    sleep 0.5
    M1="$(verdade sim.verdade)"
    descer
    sleep 0.7
    R="$("$PYTHON" -I -c '
import json,math,sys
a,b=json.loads(sys.argv[1]),json.loads(sys.argv[2])
dx,dy=b["pos"][0]-a["pos"][0],b["pos"][1]-a["pos"][1]
y=a["yaw"]
d=math.degrees(math.atan2(math.sin(b["yaw"]-y),math.cos(b["yaw"]-y)))
print(round(math.cos(y)*dx+math.sin(y)*dy,3), round(-math.sin(y)*dx+math.cos(y)*dy,3), round(d,1))' "$M0" "$M1")"
    "$PYTHON" -I -c '
import sys
frente,lado,giro=map(float,sys.argv[1].split())
sys.exit(0 if eval(sys.argv[2]) else 1)' "$R" "$6" || falha "$1: $R (frente, lado, giro), queria $6"
    echo "PASSOU $1: $R (frente, lado, giro)"
}
mede "livre, à esquerda" modo_livre 0 0.3 0 "lado > 0.2 and abs(frente) < 0.1"
mede "livre, giro à esquerda" modo_livre 0 0 1 "giro > 40"
mede "torre segue, à direita" torre_segue_chassi 0 -0.3 0 "lado < -0.2 and abs(frente) < 0.1"
mede "chassi segue, à esquerda" chassi_segue_torre 0 0.3 0 "lado > 0.2 and abs(frente) < 0.1"
mede "chassi segue, à direita" chassi_segue_torre 0 -0.3 0 "lado < -0.2 and abs(frente) < 0.1"
mede "chassi segue, giro à esquerda" chassi_segue_torre 0 0 1 "giro > 20"
# Um alvo da torre, uma vez: o chassi gira até ela e para, sem correr atrás de um alvo que anda junto. 0,3 rad são 17°, e na
# velocidade de torre da tabela ela chega bem antes do fim.
HEAD_YAW=0.3 mede "chassi segue, alvo da torre" chassi_segue_torre 0 0 0 "10 < giro < 20"

echo "--- sair armado para o robô"
subir sigterm rede
sleep 0.3
"$PYTHON" -I -c '
import json, socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.sendall(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"robot.enable\",\"params\":{\"on\":true}}\n")
fim = time.time() + 1
while time.time() < fim:
    s.sendall(b"{\"jsonrpc\":\"2.0\",\"method\":\"robot.move\",\"params\":{\"vx\":0.5,\"vy\":0,\"vyaw\":0}}\n")
    time.sleep(0.1)
' "$T/sigterm.sock" 2>/dev/null &
PILOTO=$!
sleep 0.8
descer
# Antes do prazo do corpo simulado, de 0,5 s: quem parou foi o comando de parar, e não ele. O kill e o wait de `descer` levam
# dezenas de milissegundos.
sleep 0.2
A="$(verdade sim.verdade)"
RPM="$(campo "$A" 'max(abs(r) for r in v["rpm"])')"
ARMADO="$(campo "$A" 'v["armado"]')"
wait "$PILOTO" 2>/dev/null || true
[ "$RPM" -lt 15 ] || falha "saiu andando: $RPM rpm"
[ "$ARMADO" = True ] || falha "o corpo já tinha desarmado, então isto mediu o prazo dele, e não o rms1d"
echo "PASSOU sair armado para as rodas: $RPM rpm, antes do prazo do corpo"
sleep 0.7

echo "--- o mesmo roteiro manda os mesmos quadros em outro corpo"
subir falso falso
"$ROTEIRO" --socket "$T/falso.sock" > /dev/null || falha "roteiro contra o corpo falso"
descer
"$ROTEIRO" --comparar "$T/sim.quadros" "$T/falso.quadros" || falha "os quadros diferem entre os corpos"

echo "--- a verdade do simulador: o robô anda para onde o cliente pediu"
subir verdade rede
# O rms1d observa 2,5 s antes de assinar: a primeira leitura só vale com a telemetria chegando.
n=0
until [ "$(campo "$(tele "$T/verdade.sock")" 'v["link"]["state"] == "receiving" and v["link"]["fresh"]')" = True ]; do
    n=$((n + 1)); [ "$n" -lt 100 ] || falha "a telemetria não chegou em 10 s"
    sleep 0.1
done
A0="$(verdade sim.verdade)"
B0="$(tele "$T/verdade.sock")"
"$ROTEIRO" --socket "$T/verdade.sock" --so-andar --velocidade 0.5 || falha "andar"
sleep 0.3
A1="$(verdade sim.verdade)"
B1="$(tele "$T/verdade.sock")"
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
# A mesma distância pela telemetria: o deslocamento que o rms1d leu é o que a física fez.
TELE="$("$PYTHON" -I -c '
import json,math,sys
a,b=json.loads(sys.argv[1]),json.loads(sys.argv[2])
if not b["link"]["fresh"]:
    sys.exit("telemetria velha no fim: %s" % b["link"])
print(round(math.hypot(b["body"]["position_m"][0]-a["body"]["position_m"][0], b["body"]["position_m"][1]-a["body"]["position_m"][1]),3))' "$B0" "$B1")" || falha "a telemetria não deu o deslocamento"
"$PYTHON" -I -c '
import sys
f=float(sys.argv[1].split()[0]); t=float(sys.argv[2])
sys.exit(0 if abs(f-t)<0.03 else 1)' "$FRENTE" "$TELE" || falha "a telemetria leu $TELE m, a física andou $FRENTE"
echo "PASSOU a telemetria leu o mesmo deslocamento: $TELE m"
RPM="$(campo "$A1" 'max(abs(r) for r in v["rpm"])')"
[ "$RPM" -lt 15 ] || falha "ficou com rodas girando: $RPM rpm"
echo "PASSOU parou"
descer

echo "RMS1D SIM OK"
