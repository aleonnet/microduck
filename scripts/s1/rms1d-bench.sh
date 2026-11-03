#!/usr/bin/env bash
# Ativa e devolve o robô real. É o que os botões "Ativar robô" e "Voltar ao modo DJI" da página rodam: cada passo sai numa
# linha `[rms1d-bench]`, que a página mostra, e a falha, em `[rms1d-bench] PAROU: <motivo>`. Não comanda nada que um cliente
# real não comande: quem manda é a página, `robotctl` e `rms1d-roteiro`, pelo socket. Só muda o corpo, em `deploy/s1/rms1d.toml`.
#
#   scripts/s1/rms1d-bench.sh up       acesso, envia, para só o que segura a serial, sobe o `rms1d`. Nada se move. De novo:
#                                      o anterior sai primeiro, e a gravação dele fica em `out/`.
#   scripts/s1/rms1d-bench.sh volta    guarda a gravação, reinicia o robô e confere que a DJI voltou.
#   scripts/s1/rms1d-bench.sh solta    só neste Mac: encerra a ponte e a porta até o robô. O `rms1d` dele segue no ar.
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
TCP="${S1_PORTA_ROBO:-9010}"
TOML="${S1_TOML:-deploy/s1/rms1d.toml}"
ROBOTCTL="${S1_ROBOTCTL:-target/debug/robotctl}"
BB=/system/xbin/busybox
ENTRA="$BB chroot $CHROOT /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
BIN=target/armv7-unknown-linux-gnueabihf/release/rms1d
A=(adb -s "$ALVO")
# Prova do reinício: `/tmp` é tmpfs (`init.rc`, `mount tmpfs tmpfs /tmp`), e só reiniciar apaga a marca.
MARCA=/tmp/rms1d_antes_do_reinicio
SOCKET="$ESTADO/robot.sock"

diz() { echo "[rms1d-bench] $*"; }
falha() { echo "[rms1d-bench] PAROU: $*" >&2; exit 1; }

# `adb shell` do Android 4.4 não devolve o código de saída do comando: toda conferência no robô é por uma palavra na saída.
confere() { # confere <palavra> <mensagem> <comando no robô>
    local saida
    saida="$("${A[@]}" shell "$3" || true)"
    echo "$saida"
    case "$saida" in
        *"$1"*) ;;
        *) falha "$2" ;;
    esac
}

# Uma gravação por processo: quem encerra o `rms1d` guarda a dele, ou o seguinte a sobrescreve.
guardar_gravacao() { # guardar_gravacao <nome em $OUT>
    local g
    g="$(python3 -I -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["corpo"].get("gravar",""))' "$TOML")"
    if [ -n "$g" ]; then
        if "${A[@]}" pull "$CHROOT$g" "$OUT/$1" >/dev/null 2>&1; then diz "gravação anterior guardada em out/"; else diz "sem gravação anterior para guardar"; fi
    fi
}

up() {
    mkdir -p "$ESTADO" "$OUT"
    if [ -z "${S1_SEM_COMPILAR:-}" ]; then
        diz "compilando para o robô"
        cargo s1 -p rms1d
        cargo build -q -p rms1d -p robotctl
    fi
    # Antes de qualquer coisa no robô: o arquivo recusado, com dji_scratch por exemplo, não chega a parar nada.
    local parar
    parar="$(target/debug/rms1d --config "$TOML" --processos-a-parar)" || falha "$TOML recusado; nada foi tocado no robô"
    diz "abrindo o acesso ao robô em $IP"
    python3 -I "$ACESSO" abrir --host "$IP"
    confere chroot_ok "sem chroot em $CHROOT; rode scripts/s1/bench.sh exec uma vez" "test -d $CHROOT/bin && echo chroot_ok"
    "${A[@]}" shell "$BB grep -q ' $CHROOT/dev ' /proc/mounts || mount -o bind /dev $CHROOT/dev; $BB grep -q ' $CHROOT/proc ' /proc/mounts || mount -t proc proc $CHROOT/proc; $BB grep -q ' $CHROOT/sys ' /proc/mounts || mount -t sysfs sys $CHROOT/sys"
    "${A[@]}" shell "mkdir -p $CHROOT/opt/robot/bin $CHROOT/etc/robot $CHROOT/run $CHROOT/var/lib/robot"
    # Um `rms1d` de antes sai pelo caminho que para o robô, e antes do envio: binário em uso não se sobrescreve, e a trava
    # faria o novo morrer em silêncio, com o velho respondendo no lugar dele.
    confere sem_rms1d "o rms1d anterior não saiu; desligue o robô na chave" "$BB pkill -TERM rms1d; $BB sleep 1; $BB pidof rms1d >/dev/null && echo RMS1D_ANTIGO_VIVO || echo sem_rms1d"
    guardar_gravacao "robo-$(date +%H%M%S).quadros"
    diz "enviando o rms1d ao robô"
    "${A[@]}" push "$BIN" "$CHROOT/opt/robot/bin/rms1d" >/dev/null
    "${A[@]}" push "$TOML" "$CHROOT/etc/robot/rms1d.toml" >/dev/null
    "${A[@]}" shell "chmod 755 $CHROOT/opt/robot/bin/rms1d"
    local lista="" vivos=""
    for p in $parar; do
        lista="$lista stop $p; setprop ctl.stop $p; kill \$($BB pidof $p) 2>/dev/null;"
        vivos="$vivos $BB pidof $p >/dev/null && echo AINDA_NO_AR_$p;"
    done
    diz "parando: $parar. A telemetria da DJI segue."
    local saida
    saida="$("${A[@]}" shell "$lista $BB sleep 1; $vivos for f in /proc/[0-9]*/fd/*; do $BB readlink \$f 2>/dev/null | $BB grep -q ttyS3 && echo SERIAL_PRESA \$f; done; echo parada_conferida" | tee "$OUT/parar.txt")"
    echo "$saida"
    case "$saida" in *parada_conferida*) ;; *) falha "a parada dos processos não respondeu" ;; esac
    case "$saida" in *AINDA_NO_AR*|*SERIAL_PRESA*) falha "a serial não ficou livre: dois escritores nela é o que este passo evita. Reinicie o robô" ;; esac
    diz "subindo o rms1d no robô"
    "${A[@]}" shell "($BB setsid $BB nohup $ENTRA /opt/robot/bin/rms1d --config /etc/robot/rms1d.toml --socket /run/robotd.sock </dev/null >/data/rms1d.log 2>&1 &); $BB sleep 1"
    sleep 2
    "${A[@]}" shell "$BB pidof rms1d >/dev/null && echo rms1d_no_ar || { echo RMS1D_MORTO; $BB tail -5 /data/rms1d.log; }" | tee "$OUT/up.txt"
    grep -q rms1d_no_ar "$OUT/up.txt" || falha "o rms1d não subiu; o motivo está acima"
    "${A[@]}" forward "tcp:$TCP" "localfilesystem:$CHROOT/run/robotd.sock" >/dev/null
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    nohup python3 -I scripts/s1/rms1d-ponte.py "$SOCKET" "127.0.0.1:$TCP" >"$ESTADO/ponte.log" 2>&1 &
    echo $! > "$ESTADO/ponte.pid"
    sleep 1
    diz "conferindo a saúde do rms1d"
    local rc=0
    "$ROBOTCTL" --robot-socket "$SOCKET" --config-socket "$SOCKET" --socket "$SOCKET" health > "$OUT/saude.txt" 2>&1 || rc=$?
    cat "$OUT/saude.txt"
    if [ "$rc" != 0 ]; then
        # Serial muda e telemetria recusada são o que esta prova quer saber, e dá para pilotar sem elas: avisa e segue. Qualquer
        # outra coisa para.
        grep -q "mudo\|telemetria" "$OUT/saude.txt" || falha "o rms1d está doente"
        diz "AVISO: $(grep "robot " "$OUT/saude.txt" | head -1). Se as rodas também não obedecerem, comece por aqui."
    fi
    diz "no ar, desarmado. Nada se moveu."
}

solta() {
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP" >/dev/null 2>&1 || true
}

volta() {
    mkdir -p "$OUT"
    # Quem chama a volta pode estar com o `adb` caído, depois de o Wi-Fi cair ou de o robô ser religado na chave. Sem reconectar,
    # nada abaixo chega ao robô, e o reinício não acontece.
    diz "conectando ao robô"
    local saida
    # Sem buffer: o erro, que vai para stderr, tem de sair depois do resto, porque a última linha é o motivo.
    saida="$(python3 -I -u "$ACESSO" abrir --host "$IP" 2>&1)" || {
        echo "$saida"
        falha "não consegui abrir o acesso ao robô em $IP: $(printf '%s\n' "$saida" | tail -1). O runbook do acesso diz o que fazer"
    }
    diz "parando o rms1d no robô"
    # SIGTERM: o `rms1d` para o robô e descarrega a gravação antes de sair.
    "${A[@]}" shell "$BB pkill -TERM rms1d; $BB sleep 1" || true
    guardar_gravacao "robo-$(date +%H%M%S).quadros"
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP" >/dev/null 2>&1 || true
    confere marca_gravada "não consegui marcar o robô antes de reiniciar; o robô não foi reiniciado. Tente de novo" "echo x > $MARCA && echo marca_gravada"
    diz "reiniciando o robô"
    "${A[@]}" shell reboot || true
    diz "esperando o robô voltar"
    # Sem espera fixa: só vale o robô que reiniciou de fato, sem a marca, e com a DJI no ar.
    local limite=$((SECONDS + ${S1_ESPERA_REBOOT:-180}))
    while :; do
        sleep "${S1_PASSO_REBOOT:-5}"
        if python3 -I "$ACESSO" abrir --host "$IP" >/dev/null 2>&1; then
            case "$("${A[@]}" shell "$BB test -e $MARCA && echo marca_viva || echo marca_sumiu" 2>/dev/null || true)" in
                *marca_sumiu*)
                    "${A[@]}" shell "$BB pidof dji_hdvt_uav && echo dji_voltou || echo DJI_NAO_VOLTOU" > "$OUT/volta.txt" || true
                    if grep -q dji_voltou "$OUT/volta.txt"; then break; fi
                    ;;
            esac
        fi
        [ "$SECONDS" -lt "$limite" ] || falha "a DJI não voltou em ${S1_ESPERA_REBOOT:-180} s; desligue na chave e ligue de novo"
    done
    diz "a DJI voltou. Lembre: só desligar na chave fecha a porta root."
}

case "${1:-}" in
    up) up ;;
    volta) volta ;;
    solta) solta ;;
    *) sed -n 2,10p "$0"; exit 2 ;;
esac
