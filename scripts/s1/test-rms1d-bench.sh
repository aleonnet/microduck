#!/usr/bin/env bash
# Os dois passos da prova contra `adb` e acesso falsos: pede só o previsto, e nunca o que derruba o robô.
# O que este teste não pega é o que só o robô responde.
set -euo pipefail
cd "$(dirname "$0")/../.."
T="$(mktemp -d /tmp/rms1b.XXXXXX)"
parar_ponte() { if [ -f "$T/e/ponte.pid" ]; then kill "$(cat "$T/e/ponte.pid")" 2>/dev/null || true; fi; }
trap parar_ponte EXIT
mkdir -p "$T/bin" "$T/e" "$T/out"
export ADB_LOG="$T/adb.log"; : > "$ADB_LOG"
cat > "$T/bin/adb" <<'SHIM'
#!/bin/sh
echo "$*" >> "$ADB_LOG"
case "$*" in
  *pidof\ rms1d*) echo rms1d_no_ar ;;
  *pidof\ dji_hdvt_uav*) echo dji_voltou ;;
esac
exit 0
SHIM
chmod +x "$T/bin/adb"
cat > "$T/acesso.py" <<'SHIM'
import sys
print("acesso", sys.argv[1:])
SHIM
export PATH="$T/bin:$PATH" S1_ACESSO="$T/acesso.py" S1_SEM_COMPILAR=1 S1_ESTADO="$T/e" S1_OUT="$T/out" S1_ESPERA_REBOOT=0
falhou=0
pediu() { if grep -qF -- "$1" "$ADB_LOG"; then echo "pediu: $1"; else echo "NÃO PEDIU: $1"; falhou=1; fi; }
nunca() { if grep -qF -- "$1" "$ADB_LOG"; then echo "PEDIU O PROIBIDO: $1"; falhou=1; else echo "nunca: $1"; fi; }

# A saúde, em `up`, fala com um socket onde não há robô, e reprovar ali é esperado: aqui só importa o que foi pedido.
cargo build -q -p robotctl
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || true
grep -q "rms1d_no_ar" "$T/up.txt" || { echo "up não chegou ao rms1d:"; tail -20 "$T/up.txt"; exit 1; }
pediu "stop dji_hdvt_uav"
pediu "stop dji_monitor"
pediu "/opt/robot/bin/rms1d --config /etc/robot/rms1d.toml --socket /run/robotd.sock"
pediu "forward tcp:9000 localfilesystem:/data/debian/run/robotd.sock"
pediu "push deploy/s1/rms1d.toml /data/debian/etc/robot/rms1d.toml"
scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 || { echo "volta falhou:"; tail -20 "$T/volta.txt"; exit 1; }
pediu "shell reboot"
pediu "forward --remove tcp:9000"
pediu "pidof dji_hdvt_uav"
nunca "dji_scratch"
nunca "dji_sys"
nunca "dji_network"
nunca "dji_camera"
nunca "dd if="
nunca "remount"
[ "$falhou" = 0 ] || { echo "BANCADA FALSA REPROVOU"; exit 1; }
echo "BANCADA FALSA OK"
