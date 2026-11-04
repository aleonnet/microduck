#!/usr/bin/env bash
# Ativa e devolve o robô real. É o que os botões "Ativar robô" e "Voltar ao modo DJI" da página rodam: cada passo sai numa
# linha `[rms1d-bench]`, que a página mostra, e a falha, em `[rms1d-bench] PAROU: <motivo>`. Não comanda nada que um cliente
# real não comande: quem manda é a página, `robotctl` e `rms1d-roteiro`, pelo socket. Só muda o corpo, em `deploy/s1/rms1d.toml`.
#
#   scripts/s1/rms1d-bench.sh up       acesso, envia, para só o que segura a serial, sobe o `rms1d`. Nada se move. De novo:
#                                      o anterior sai primeiro, e a gravação dele fica em $S1_OUT. O binário só vai ao
#                                      robô se mudou.
#   scripts/s1/rms1d-bench.sh volta    guarda a gravação, reinicia o robô e confere que a DJI voltou.
#   scripts/s1/rms1d-bench.sh solta    só neste Mac: encerra a ponte e a porta até o robô. O `rms1d` dele segue no ar.
#
# Ambiente: S1_HOST (padrão 192.168.1.236), S1_LAB (padrão ~/Development/RoboMaster-S1), S1_SEM_COMPILAR=1, S1_OUT.
#
# Cada passo termina com `[rms1d-bench] TEMPO <ms> <texto do passo>`, que a página mostra ao lado do passo.
set -euo pipefail
cd "$(dirname "$0")/../.."

IP="${S1_HOST:-192.168.1.236}"
ALVO="$IP:5555"
LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
ACESSO="${S1_ACESSO:-$LAB/tools/acesso/s1_acesso.py}"
CHROOT="${S1_CHROOT:-/data/debian}"
ESTADO="${S1_ESTADO:-/tmp/rms1d-bench}"
# As gravações de cada ativação ficam fora dos repositórios, no lugar de estado do usuário da especificação XDG ("state data
# that should persist between (application) restarts", `$XDG_STATE_HOME`, padrão `~/.local/state`). Evidência entra no
# laboratório por escolha, em `out/<data>-<assunto>/`: gravar ali a cada ativação sujava a árvore com arquivos de ninguém.
OUT="${S1_OUT:-${XDG_STATE_HOME:-$HOME/.local/state}/rms1/gravacoes/$(date +%Y-%m-%d)}"
TCP="${S1_PORTA_ROBO:-9010}"
TOML="${S1_TOML:-deploy/s1/rms1d.toml}"
ROBOTCTL="${S1_ROBOTCTL:-target/debug/robotctl}"
BB=/system/xbin/busybox
ENTRA="$BB chroot $CHROOT /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
BIN="${S1_BIN:-target/armv7-unknown-linux-gnueabihf/release/rms1d}"
A=(adb -s "$ALVO")
# Prova do reinício: `/tmp` é tmpfs (`init.rc`, `mount tmpfs tmpfs /tmp`), e só reiniciar apaga a marca.
MARCA=/tmp/rms1d_antes_do_reinicio
SOCKET="$ESTADO/robot.sock"

diz() { echo "[rms1d-bench] $*"; }
agora_ms() { perl -MTime::HiRes=time -e 'printf "%d", time * 1000'; }
DESDE=""
ATUAL=""
# Um passo novo fecha o anterior com `TEMPO <ms> <texto do passo>`; `passo` sem texto só fecha.
passo() {
    local t
    t="$(agora_ms)"
    if [ -n "$DESDE" ]; then diz "TEMPO $((t - DESDE)) $ATUAL"; fi
    DESDE="$t"
    ATUAL="$*"
    if [ $# -gt 0 ]; then diz "$*"; else DESDE=""; fi
}
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

# Os caminhos das duas gravações no robô, em `rms1d.toml`: o que o `rms1d` mandou (`gravar`) e o que ele recebeu
# (`gravar_recebido`). Uma por processo: o seguinte a sobrescreve.
gravacoes() {
    python3 -I -c 'import sys,tomllib; c=tomllib.load(open(sys.argv[1],"rb"))["corpo"]; print(c.get("gravar","")); print(c.get("gravar_recebido",""))' "$TOML"
}

# Volta: copia as gravações do `rms1d` que acabou de sair. É o fim da sessão, e quem espera é o reinício.
guardar_gravacao() { # guardar_gravacao <prefixo em $OUT>
    local g r
    { read -r g; read -r r; } < <(gravacoes)
    if [ -n "$g" ]; then "${A[@]}" pull "$CHROOT$g" "$OUT/$1.quadros" >/dev/null 2>&1 || true; fi
    if [ -n "$r" ]; then "${A[@]}" pull "$CHROOT$r" "$OUT/$1.recebido.txt" >/dev/null 2>&1 || true; fi
    echo "gravação anterior em $OUT"
}

# Ativação: no robô, as gravações do `rms1d` anterior só mudam de nome, o que é instantâneo, para um nome desta ativação, que
# nenhuma outra sobrescreve. A cópia pelo Wi-Fi, que chegou a 18 s, começa logo, ao fundo, e só apaga a do robô depois de chegar:
# uma falha adiante na ativação, ou uma ativação seguinte, não perde a gravação (o leitor frio, 2026-11-04: com um `.anterior`
# fixo e a cópia só depois do "Robô pronto", uma subida que parava no meio deixava a gravação para a próxima apagar).
separar_gravacao() { # separar_gravacao <prefixo em $OUT>
    local g r
    { read -r g; read -r r; } < <(gravacoes)
    for f in $g $r; do "${A[@]}" shell "$BB test -f $CHROOT$f && $BB mv -f $CHROOT$f $CHROOT$f.$1" >/dev/null 2>&1 || true; done
    (
        if [ -n "$g" ] && "${A[@]}" pull "$CHROOT$g.$1" "$OUT/$1.quadros"; then "${A[@]}" shell "$BB rm -f $CHROOT$g.$1"; fi
        if [ -n "$r" ] && "${A[@]}" pull "$CHROOT$r.$1" "$OUT/$1.recebido.txt"; then "${A[@]}" shell "$BB rm -f $CHROOT$r.$1"; fi
    ) >"$ESTADO/copia.log" 2>&1 </dev/null &
    echo $! > "$ESTADO/copia.pid"
}

# A cópia ao fundo termina antes de outra ativação ou da volta: o reinício a cortaria no meio. Confere que o processo é a cópia,
# e não outro que herdou o número. Com o prazo vencido, segue: a gravação fica no robô, com o nome da ativação dela.
esperar_copia() {
    local pid limite=$((SECONDS + ${S1_ESPERA_COPIA:-120}))
    [ -f "$ESTADO/copia.pid" ] || return 0
    pid="$(cat "$ESTADO/copia.pid")"
    while ps -p "$pid" -o command= 2>/dev/null | grep -q rms1d-bench; do
        if [ "$SECONDS" -ge "$limite" ]; then
            echo "a cópia da gravação anterior não terminou em ${S1_ESPERA_COPIA:-120} s; ela fica no robô, com o nome da ativação dela"
            return 0
        fi
        sleep 0.2
    done
}

# A página até o `rms1d` do robô: a porta do `adb` e a ponte do socket.
ligar_ponte() {
    "${A[@]}" forward "tcp:$TCP" "localfilesystem:$CHROOT/run/robotd.sock" >/dev/null
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    nohup python3 -I scripts/s1/rms1d-ponte.py "$SOCKET" "127.0.0.1:$TCP" >"$ESTADO/ponte.log" 2>&1 </dev/null &
    echo $! > "$ESTADO/ponte.pid"
    sleep 1
}

saude() { # saude: 0 saudável; 1 doente; o texto em $OUT/saude.txt
    local rc=0
    "$ROBOTCTL" --robot-socket "$SOCKET" --config-socket "$SOCKET" --socket "$SOCKET" health > "$OUT/saude.txt" 2>&1 || rc=$?
    cat "$OUT/saude.txt"
    return "$rc"
}

# Três caminhos, pelo que o robô tem: nada nosso (preparar), o mesmo `rms1d` no ar e saudável (só conectar, cerca de 1 s) e outro
# `rms1d` (atualizar, sem parar de novo a DJI, que já está parada). Na tela, só os passos de quem usa; o resto vai ao registro.
up() {
    mkdir -p "$ESTADO" "$OUT"
    esperar_copia
    if [ -z "${S1_SEM_COMPILAR:-}" ]; then
        passo "Compilando o rms1d"
        cargo s1 -p rms1d
        cargo build -q -p rms1d -p robotctl
    fi
    # Antes de qualquer coisa no robô: o arquivo recusado, com dji_scratch por exemplo, não chega a parar nada.
    local parar
    parar="$(target/debug/rms1d --config "$TOML" --processos-a-parar)" || falha "$TOML recusado; nada foi tocado no robô"
    # Sem o binário, a soma vazia daria "igual" à do robô sem binário, e a DJI seria parada antes da falha.
    [ -s "$BIN" ] || falha "não achei $BIN; compile com cargo s1 -p rms1d. Nada foi tocado no robô"
    passo "Abrindo o acesso ao robô"
    python3 -I "$ACESSO" abrir --host "$IP"
    confere chroot_ok "sem chroot em $CHROOT; rode scripts/s1/bench.sh exec uma vez" "test -d $CHROOT/bin && echo chroot_ok"
    "${A[@]}" shell "$BB grep -q ' $CHROOT/dev ' /proc/mounts || mount -o bind /dev $CHROOT/dev; $BB grep -q ' $CHROOT/proc ' /proc/mounts || mount -t proc proc $CHROOT/proc; $BB grep -q ' $CHROOT/sys ' /proc/mounts || mount -t sysfs sys $CHROOT/sys"
    "${A[@]}" shell "mkdir -p $CHROOT/opt/robot/bin $CHROOT/etc/robot $CHROOT/run $CHROOT/var/lib/robot"
    # O que o robô tem. O busybox dele tem `sha256sum` (lido na cópia completa, `arvores/system/system/xbin/busybox`); o
    # `adb shell` do Android 4.4 termina a linha com `\r`.
    local estado soma_bin soma_toml vivo=""
    estado="$("${A[@]}" shell "echo estado_do_robo; $BB pidof rms1d >/dev/null && echo RMS1D_VIVO; $BB sha256sum $CHROOT/opt/robot/bin/rms1d $CHROOT/etc/robot/rms1d.toml 2>/dev/null" | tr -d '\r' || true)"
    echo "$estado"
    case "$estado" in *RMS1D_VIVO*) vivo=1 ;; esac
    soma_bin="$(printf '%s\n' "$estado" | grep '/opt/robot/bin/rms1d$' | cut -d' ' -f1 || true)"
    soma_toml="$(printf '%s\n' "$estado" | grep '/etc/robot/rms1d.toml$' | cut -d' ' -f1 || true)"
    local mesmo_bin="" mesmo_toml=""
    if [ -n "$soma_bin" ] && [ "$soma_bin" = "$(shasum -a 256 "$BIN" | cut -d' ' -f1)" ]; then mesmo_bin=1; fi
    if [ -n "$soma_toml" ] && [ "$soma_toml" = "$(shasum -a 256 "$TOML" | cut -d' ' -f1)" ]; then mesmo_toml=1; fi
    if [ -n "$vivo" ] && [ -n "$mesmo_bin" ] && [ -n "$mesmo_toml" ]; then
        passo "Conectando ao robô"
        ligar_ponte
        if saude; then
            passo
            diz "Robô pronto"
            return
        fi
        echo "o rms1d no robô não respondeu bem: preparando de novo"
    fi
    if [ -n "$vivo" ]; then passo "Atualizando o robô"; else passo "Preparando o robô"; fi
    # Um `rms1d` de antes sai pelo caminho que para o robô, e antes do envio: binário em uso não se sobrescreve, e a trava
    # faria o novo morrer em silêncio, com o velho respondendo no lugar dele.
    confere sem_rms1d "o rms1d anterior não saiu; desligue o robô na chave" "$BB pkill -TERM rms1d; $BB sleep 1; $BB pidof rms1d >/dev/null && echo RMS1D_ANTIGO_VIVO || echo sem_rms1d"
    separar_gravacao "robo-$(date +%H%M%S)"
    if [ -z "$mesmo_bin" ]; then "${A[@]}" push "$BIN" "$CHROOT/opt/robot/bin/rms1d" >/dev/null; fi
    "${A[@]}" push "$TOML" "$CHROOT/etc/robot/rms1d.toml" >/dev/null
    "${A[@]}" shell "chmod 755 $CHROOT/opt/robot/bin/rms1d"
    # Os processos da DJI que seguram a serial: parados só se estiverem no ar, e a serial conferida livre sempre.
    local lista="" vivos=""
    for p in $parar; do
        lista="$lista $BB pidof $p >/dev/null && { stop $p; setprop ctl.stop $p; kill \$($BB pidof $p) 2>/dev/null; $BB sleep 1; };"
        vivos="$vivos $BB pidof $p >/dev/null && echo AINDA_NO_AR_$p;"
    done
    local saida
    saida="$("${A[@]}" shell "$lista $vivos for f in /proc/[0-9]*/fd/*; do $BB readlink \$f 2>/dev/null | $BB grep -q ttyS3 && echo SERIAL_PRESA \$f; done; echo parada_conferida" | tee "$OUT/parar.txt")"
    echo "$saida"
    case "$saida" in *parada_conferida*) ;; *) falha "a parada dos processos da DJI não respondeu" ;; esac
    case "$saida" in *AINDA_NO_AR*|*SERIAL_PRESA*) falha "a serial do controlador não ficou livre: dois programas nela é o que este passo evita. Reinicie o robô" ;; esac
    "${A[@]}" shell "($BB setsid $BB nohup $ENTRA /opt/robot/bin/rms1d --config /etc/robot/rms1d.toml --socket /run/robotd.sock </dev/null >/data/rms1d.log 2>&1 &); $BB sleep 1"
    sleep 2
    "${A[@]}" shell "$BB pidof rms1d >/dev/null && echo rms1d_no_ar || { echo RMS1D_MORTO; $BB tail -5 /data/rms1d.log; }" | tee "$OUT/up.txt"
    grep -q rms1d_no_ar "$OUT/up.txt" || falha "o rms1d não subiu no robô; o motivo está no registro do serviço"
    ligar_ponte
    if ! saude; then
        # Serial muda e telemetria recusada não impedem pilotar: avisa e segue. Qualquer outra coisa para.
        if grep -q "mudo" "$OUT/saude.txt"; then
            diz "AVISO: nada chegou do controlador. Se as rodas também não obedecerem, comece por aqui."
        elif grep -q "telemetria" "$OUT/saude.txt"; then
            diz "AVISO: sem telemetria ($(grep "robot " "$OUT/saude.txt" | head -1 | sed 's/^robot *//')). Dá para pilotar; o gêmeo e o painel ficam sem dados."
        else
            falha "o rms1d está doente"
        fi
    fi
    passo
    diz "Robô pronto"
}

solta() {
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP" >/dev/null 2>&1 || true
}

volta() {
    mkdir -p "$OUT"
    # Quem chama a volta pode estar com o `adb` caído, depois de o Wi-Fi cair ou de o robô ser religado na chave. Sem reconectar,
    # nada abaixo chega ao robô, e o reinício não acontece.
    passo "conectando ao robô"
    local saida
    # Sem buffer: o erro, que vai para stderr, tem de sair depois do resto, porque a última linha é o motivo.
    saida="$(python3 -I -u "$ACESSO" abrir --host "$IP" 2>&1)" || {
        echo "$saida"
        falha "não consegui abrir o acesso ao robô em $IP: $(printf '%s\n' "$saida" | tail -1). O runbook do acesso diz o que fazer"
    }
    esperar_copia
    passo "parando o rms1d no robô"
    # SIGTERM: o `rms1d` para o robô e descarrega a gravação antes de sair.
    "${A[@]}" shell "$BB pkill -TERM rms1d; $BB sleep 1" || true
    guardar_gravacao "robo-$(date +%H%M%S)"
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP" >/dev/null 2>&1 || true
    confere marca_gravada "não consegui marcar o robô antes de reiniciar; o robô não foi reiniciado. Tente de novo" "echo x > $MARCA && echo marca_gravada"
    passo "reiniciando o robô"
    "${A[@]}" shell reboot || true
    passo "esperando o robô voltar"
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
    passo
    diz "a DJI voltou. Lembre: só desligar na chave fecha a porta root."
}

case "${1:-}" in
    up) up ;;
    volta) volta ;;
    solta) solta ;;
    *) sed -n 2,10p "$0"; exit 2 ;;
esac
