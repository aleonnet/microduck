#!/usr/bin/env bash
# Prepara e encerra a prova no robô real. Não comanda nada que um cliente real não comande: quem manda é o
# console, `robotctl` e `rms1d-roteiro`, pelo socket, como no simulador. Só muda o corpo, em `deploy/s1/rms1d.toml`.
#
#   scripts/s1/rms1d-bench.sh up       acesso, envia, para só o que segura a serial, sobe o `rms1d`. Nada se move.
#   scripts/s1/rms1d-bench.sh console  `mediad` neste Mac, com padrão de teste, e abre o console.
#   scripts/s1/rms1d-bench.sh volta    guarda a gravação, reinicia o robô e confere que a DJI voltou.
#
# Ambiente: S1_HOST (padrão 192.168.1.236), S1_LAB (padrão ~/Development/RoboMaster-S1), S1_SEM_COMPILAR=1.
set -euo pipefail
cd "$(dirname "$0")/../.."

IP="${S1_HOST:-192.168.1.236}"
ALVO="$IP:5555"
LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
ACESSO="${S1_ACESSO:-$LAB/tools/acesso/s1_acesso.py}"
CHROOT="${S1_CHROOT:-/data/debian}"
ESTADO="${S1_ESTADO:-/tmp/rms1d-bench}"
OUT="${S1_OUT:-$LAB/out/$(date +%Y-%m-%d)-rms1d-chassi-torre}"
TCP=9000
TOML=deploy/s1/rms1d.toml
BB=/system/xbin/busybox
ENTRA="$BB chroot $CHROOT /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
BIN=target/armv7-unknown-linux-gnueabihf/release/rms1d
A=(adb -s "$ALVO")
SOCKET="$ESTADO/robot.sock"

diz() { echo "[rms1d-bench] $*"; }
falha() { echo "[rms1d-bench] PAROU: $*" >&2; exit 1; }

# Os processos a parar vêm do arquivo, validado pelo `rms1d`: dji_scratch e dji_sys são recusados lá.
processos() {
    python3 -I -c 'import sys,tomllib; print(" ".join(tomllib.load(open(sys.argv[1],"rb"))["dji"]["parar"]))' "$TOML"
}

up() {
    mkdir -p "$ESTADO" "$OUT"
    if [ -z "${S1_SEM_COMPILAR:-}" ]; then
        diz "compilando para o robô"
        cargo s1 -p rms1d
        cargo build -q -p rms1d -p robotctl
    fi
    python3 -I "$ACESSO" abrir --host "$IP"
    "${A[@]}" shell "test -d $CHROOT/bin" || falha "sem chroot em $CHROOT; rode scripts/s1/bench.sh exec uma vez"
    "${A[@]}" shell "$BB grep -q ' $CHROOT/dev ' /proc/mounts || mount -o bind /dev $CHROOT/dev; $BB grep -q ' $CHROOT/proc ' /proc/mounts || mount -t proc proc $CHROOT/proc; $BB grep -q ' $CHROOT/sys ' /proc/mounts || mount -t sysfs sys $CHROOT/sys"
    "${A[@]}" shell "mkdir -p $CHROOT/opt/robot/bin $CHROOT/etc/robot $CHROOT/run $CHROOT/var/lib/robot"
    "${A[@]}" push "$BIN" "$CHROOT/opt/robot/bin/rms1d" >/dev/null
    "${A[@]}" push "$TOML" "$CHROOT/etc/robot/rms1d.toml" >/dev/null
    "${A[@]}" shell "chmod 755 $CHROOT/opt/robot/bin/rms1d"
    local parar lista=""
    parar="$(processos)"
    for p in $parar; do
        lista="$lista stop $p; setprop ctl.stop $p; kill \$($BB pidof $p) 2>/dev/null;"
    done
    diz "parando: $parar. A telemetria da DJI segue; a serial fica livre."
    "${A[@]}" shell "$lista $BB sleep 1"
    "${A[@]}" shell "($BB setsid $BB nohup $ENTRA /opt/robot/bin/rms1d --config /etc/robot/rms1d.toml --socket /run/robotd.sock </dev/null >/data/rms1d.log 2>&1 &); $BB sleep 1"
    sleep 2
    "${A[@]}" shell "$BB pidof rms1d >/dev/null && echo rms1d_no_ar || { echo RMS1D_MORTO; $BB tail -5 /data/rms1d.log; }" | tee "$OUT/up.txt"
    grep -q rms1d_no_ar "$OUT/up.txt" || falha "o rms1d não subiu; o motivo está acima"
    "${A[@]}" forward "tcp:$TCP" "localfilesystem:$CHROOT/run/robotd.sock" >/dev/null
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    nohup python3 -I scripts/s1/rms1d-ponte.py "$SOCKET" "127.0.0.1:$TCP" >"$ESTADO/ponte.log" 2>&1 &
    echo $! > "$ESTADO/ponte.pid"
    sleep 1
    diz "saúde, pelo socket:"
    target/debug/robotctl --robot-socket "$SOCKET" --config-socket "$SOCKET" --socket "$SOCKET" health | tee "$OUT/saude.txt" || falha "o rms1d está doente"
    diz "no ar, desarmado. Nada se moveu. Socket deste Mac: $SOCKET"
    diz "roteiro: target/debug/rms1d-roteiro --socket $SOCKET"
}

console() {
    [ -S "$SOCKET" ] || falha "sem socket; rode up primeiro"
    S1_SOCKET="$SOCKET" scripts/rms1d-sim console
}

volta() {
    mkdir -p "$OUT"
    local g
    g="$(python3 -I -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["corpo"].get("gravar",""))' "$TOML")"
    "${A[@]}" shell "$BB pkill -TERM rms1d; $BB sleep 1" || true
    if [ -n "$g" ]; then "${A[@]}" pull "$CHROOT$g" "$OUT/robo.quadros" >/dev/null || diz "sem gravação para guardar"; fi
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP" >/dev/null 2>&1 || true
    diz "reiniciando o robô"
    "${A[@]}" shell reboot || true
    sleep "${S1_ESPERA_REBOOT:-60}"
    python3 -I "$ACESSO" abrir --host "$IP"
    "${A[@]}" shell "$BB pidof dji_hdvt_uav && echo dji_voltou || echo DJI_NAO_VOLTOU" | tee "$OUT/volta.txt"
    grep -q dji_voltou "$OUT/volta.txt" || falha "a DJI não voltou; desligue na chave e ligue de novo"
    diz "a DJI voltou. Lembre: só desligar na chave fecha a porta root."
}

case "${1:-}" in
    up) up ;;
    console) console ;;
    volta) volta ;;
    *) sed -n 2,9p "$0"; exit 2 ;;
esac
