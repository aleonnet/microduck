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
# Tudo o que foi pedido nesta execução, em todos os cenários: é aqui que se procura o proibido.
export ADB_TUDO="$T/adb-tudo.log"; : > "$ADB_TUDO"
# O `adb` falso devolve sempre 0, como o do Android 4.4: quem reprova é a palavra na saída. Cada defeito do robô é um arquivo
# em $T/defeito.
export DEFEITO="$T/defeito"; : > "$DEFEITO"
cat > "$T/bin/adb" <<'SHIM'
#!/bin/sh
echo "$*" >> "$ADB_LOG"
echo "$*" >> "$ADB_TUDO"
tem() { grep -qx "$1" "$DEFEITO"; }
case "$*" in
  *"test -d"*) tem sem_chroot || echo chroot_ok ;;
  *"pkill -TERM rms1d"*sem_rms1d*) if tem rms1d_antigo; then echo RMS1D_ANTIGO_VIVO; else echo sem_rms1d; fi ;;
  *parada_conferida*) tem dji_vivo && echo AINDA_NO_AR_dji_hdvt_uav; tem serial_presa && echo SERIAL_PRESA /proc/9/fd/3; echo parada_conferida ;;
  *"pidof rms1d"*) if tem rms1d_morto; then echo RMS1D_MORTO; else echo rms1d_no_ar; fi ;;
  *"pidof dji_hdvt_uav"*) echo dji_voltou ;;
  *"/proc/uptime"*) if [ -f "$DEFEITO.reiniciou" ] && ! tem nao_reinicia; then echo "12.50 3.00"; else echo "900.50 300.00"; fi ;;
  *"shell reboot"*) touch "$DEFEITO.reiniciou" ;;
esac
exit 0
SHIM
chmod +x "$T/bin/adb"
cat > "$T/acesso.py" <<'SHIM'
import sys
print("acesso", sys.argv[1:])
SHIM
cat > "$T/bin/robotctl" <<'SHIM'
#!/bin/sh
if grep -qx serial_muda "$DEFEITO"; then echo "robot     degraded: o corpo está mudo: nenhum quadro recebido em 4 s"; exit 1; fi
if grep -qx doente "$DEFEITO"; then echo "robot     unhealthy: o laço parou"; exit 1; fi
echo "robot     healthy"
SHIM
chmod +x "$T/bin/robotctl"
export PATH="$T/bin:$PATH" S1_ACESSO="$T/acesso.py" S1_SEM_COMPILAR=1 S1_ESTADO="$T/e" S1_OUT="$T/out" S1_ESPERA_REBOOT=2 S1_PASSO_REBOOT=0.1 S1_ROBOTCTL="$T/bin/robotctl"
falhou=0
pediu() { if grep -qF -- "$1" "$ADB_LOG"; then echo "pediu: $1"; else echo "NÃO PEDIU: $1"; falhou=1; fi; }
nunca() { if grep -qF -- "$1" "$ADB_TUDO"; then echo "PEDIU O PROIBIDO: $1"; falhou=1; else echo "nunca: $1"; fi; }
antes() { # antes <primeiro> <segundo>: as duas linhas existem, nessa ordem
    p="$(grep -nF -- "$1" "$ADB_LOG" | head -1 | cut -d: -f1 || true)"; s="$(grep -nF -- "$2" "$ADB_LOG" | head -1 | cut -d: -f1 || true)"
    if [ -n "$p" ] && [ -n "$s" ] && [ "$p" -lt "$s" ]; then echo "antes: $1, depois: $2"; else echo "FORA DE ORDEM: $1 ($p), $2 ($s)"; falhou=1; fi
}
# reprova <nome> <palavra na mensagem> <o que não pode ter sido pedido>: `up` tem de parar, com a mensagem.
reprova() {
    : > "$ADB_LOG"
    if scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1; then echo "SUBIU COM DEFEITO: $1"; falhou=1; return; fi
    if grep -qF -- "$2" "$T/up.txt"; then echo "parou em $1: $2"; else echo "PAROU SEM DIZER $2 em $1:"; tail -5 "$T/up.txt"; falhou=1; fi
    if [ -n "$3" ] && grep -qF -- "$3" "$ADB_LOG"; then echo "SEGUIU ADIANTE em $1: $3"; falhou=1; fi
}

cargo build -q -p rms1d -p robotctl

echo "--- o caminho certo"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up falhou:"; tail -20 "$T/up.txt"; exit 1; }
grep -q "no ar, desarmado" "$T/up.txt" || { echo "up não terminou:"; tail -20 "$T/up.txt"; exit 1; }
pediu "stop dji_hdvt_uav"
pediu "stop dji_monitor"
pediu "/opt/robot/bin/rms1d --config /etc/robot/rms1d.toml --socket /run/robotd.sock"
pediu "forward tcp:9010 localfilesystem:/data/debian/run/robotd.sock"
pediu "push deploy/s1/rms1d.toml /data/debian/etc/robot/rms1d.toml"
antes "pkill -TERM rms1d" "push target/armv7-unknown-linux-gnueabihf/release/rms1d"
antes "stop dji_monitor" "/opt/robot/bin/rms1d --config"
antes "pkill -TERM rms1d" "pull /data/debian/var/lib/robot/rms1d-quadros.ndjson"
antes "pull /data/debian/var/lib/robot/rms1d-quadros.ndjson" "/opt/robot/bin/rms1d --config"
grep "stop dji_monitor" "$ADB_LOG" | grep -q "stop dji_monitor.*stop dji_hdvt_uav" || { echo "O MONITOR NÃO VEIO PRIMEIRO"; falhou=1; }

echo "--- o que tem de parar a subida"
printf '[dji]\nparar = ["dji_scratch"]\n' > "$T/ruim.toml"
: > "$ADB_LOG"
if S1_TOML="$T/ruim.toml" scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1; then echo "SUBIU COM dji_scratch"; falhou=1; fi
grep -qF "recusado" "$T/up.txt" || { echo "NÃO DISSE RECUSADO:"; tail -5 "$T/up.txt"; falhou=1; }
if [ -s "$ADB_LOG" ]; then echo "TOCOU O ROBÔ COM O ARQUIVO RECUSADO:"; cat "$ADB_LOG"; falhou=1; else echo "arquivo recusado: nada foi pedido ao robô"; fi

echo sem_chroot > "$DEFEITO"
reprova "sem chroot" "sem chroot" "stop dji_hdvt_uav"
echo rms1d_antigo > "$DEFEITO"
reprova "rms1d antigo vivo" "anterior não saiu" "push target"
echo dji_vivo > "$DEFEITO"
reprova "processo da DJI sobrevive" "não ficou livre" "/opt/robot/bin/rms1d --config"
echo serial_presa > "$DEFEITO"
reprova "serial presa" "não ficou livre" "/opt/robot/bin/rms1d --config"
echo rms1d_morto > "$DEFEITO"
reprova "rms1d morre ao subir" "não subiu" "forward tcp:9010"
echo doente > "$DEFEITO"
reprova "rms1d doente" "doente" ""
echo serial_muda > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "SERIAL MUDA PAROU A SUBIDA:"; tail -5 "$T/up.txt"; falhou=1; }
if grep -q "AVISO: nada chegou do corpo" "$T/up.txt"; then echo "serial muda: avisa e segue"; else echo "SERIAL MUDA SEM AVISO"; falhou=1; fi
: > "$DEFEITO"

echo "--- a volta"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 || { echo "volta falhou:"; tail -20 "$T/volta.txt"; exit 1; }
pediu "pkill -TERM rms1d"
pediu "shell reboot"
pediu "forward --remove tcp:9010"
pediu "pidof dji_hdvt_uav"
antes "/proc/uptime" "shell reboot"
echo nao_reinicia > "$DEFEITO"
rm -f "$DEFEITO.reiniciou"
if scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1; then echo "DISSE QUE VOLTOU SEM REINICIAR"; falhou=1; fi
grep -qF "a DJI não voltou" "$T/volta.txt" && echo "sem reiniciar: diz que não voltou" || { echo "SEM REINICIAR, CALADO:"; tail -3 "$T/volta.txt"; falhou=1; }
: > "$DEFEITO"
nunca "dji_scratch"
nunca "dji_sys"
nunca "dji_network"
nunca "dji_camera"
nunca "0x48"
nunca "stop dji_vision"
nunca "dd if="
nunca "remount"
[ "$falhou" = 0 ] || { echo "BANCADA FALSA REPROVOU"; exit 1; }
echo "BANCADA FALSA OK"
