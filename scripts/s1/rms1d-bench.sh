#!/usr/bin/env bash
# Ativa e devolve o robô real. É o que os botões "Ativar robô" e "Voltar ao modo DJI" da página rodam: cada passo sai numa
# linha `[rms1d-bench]`, que a página mostra, e a falha, em `[rms1d-bench] PAROU: <motivo>`. Não comanda nada que um cliente
# real não comande: quem manda é a página, `robotctl` e `rms1d-roteiro`, pelo socket. Só muda o corpo, em `deploy/s1/rms1d.toml`.
#
#   scripts/s1/rms1d-bench.sh up       acesso, envia, para só o que `[dji] parar` manda, sobe o `rms1d` e, se o arquivo tem
#                                      `[midia]`, o `rms1-mediad` (câmera, microfone e alto-falante). Nada se move. De
#                                      novo: o anterior sai primeiro, e a gravação dele fica em $S1_OUT. Cada binário só
#                                      vai ao robô se mudou.
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
# A mídia: o `rms1-mediad` no robô, e a porta e o socket por onde este Mac fala com ele.
BIN_MIDIA="${S1_BIN_MIDIA:-target/armv7-unknown-linux-gnueabihf/release/rms1-mediad}"
TCP_MIDIA="${S1_PORTA_MIDIA:-9011}"
SOCKET_MIDIA="$ESTADO/media.sock"
# Os sons do robô: os da própria DJI (`/system/audio/sound`), sob os nomes do contrato, uma pasta por som.
SONS="chirp:media_sound_demo_recognize_success greet:power_on alarm:media_sound_demo_attacked inquire:media_sound_demo_scanning peck:sleeping coo:media_sound_demo_gimbal_rotate"
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
    copiar_ao_fundo "$1"
}

# A cópia ao fundo: a gravação desta ativação (com o prefixo dado) e as esquecidas de ativações anteriores ($SOBRAS).
copiar_ao_fundo() { # copiar_ao_fundo <prefixo em $OUT, ou vazio>
    local g r
    { read -r g; read -r r; } < <(gravacoes)
    # A própria cópia grava o seu número e o apaga ao terminar: um número velho no arquivo não fica esperando outro processo.
    # O número sai do `sh` filho (o pai dele é esta cópia): o bash do macOS, 3.2, não tem `BASHPID`.
    (
        sh -c 'echo "$PPID"' > "$ESTADO/copia.pid"
        if [ -n "$1" ]; then
            if [ -n "$g" ] && "${A[@]}" pull "$CHROOT$g.$1" "$OUT/$1.quadros"; then "${A[@]}" shell "$BB rm -f $CHROOT$g.$1"; fi
            if [ -n "$r" ] && "${A[@]}" pull "$CHROOT$r.$1" "$OUT/$1.recebido.txt"; then "${A[@]}" shell "$BB rm -f $CHROOT$r.$1"; fi
        fi
        guardar_sobras "$SOBRAS"
        rm -f "$ESTADO/copia.pid"
    ) >"$ESTADO/copia.log" 2>&1 </dev/null &
}

# O espaço livre do `/data` do robô, em KiB; vazio se a resposta não se lê. O `df -k` do busybox é o que a cópia de 2026-11-01
# usou (`tools/copia/copiar_s1.py`, no laboratório): a última linha traz blocos, usado, livre, percentual e o ponto de montagem.
espaco_livre_kib() {
    "${A[@]}" shell "echo espaco_do_robo; $BB df -k /data" 2>/dev/null | tr -d '\r' | awk 'NF >= 4 && $(NF-2) ~ /^[0-9]+$/ && $NF == "/data" { v = $(NF-2) } END { print v }'
}

# As gravações de ativações anteriores que ficaram no robô (`<arquivo>.robo-<data>-<hora>`), "<bytes> <nome>" por linha. Ficam
# quando a cópia delas não chega: duas, de 62 e 39 MB, encheram o disco do robô em 2026-11-15, e o laço caiu a 2 Hz. O `ls` do
# busybox, pelo `adb shell`, sai com cores: as sequências saem aqui.
sobras_no_robo() {
    local g r
    { read -r g; read -r r; } < <(gravacoes)
    [ -n "$g" ] || return 0
    "${A[@]}" shell "echo sobras_do_robo; $BB ls -l $(dirname "$CHROOT$g")" 2>/dev/null | tr -d '\r' | sed $'s/\x1b\\[[0-9;]*m//g' | awk '$NF ~ /[.]robo-/ { print $5, $NF }'
}

# Copia cada sobra, confere a soma SHA-256 dos dois lados e só então a apaga no robô. A que não copia, ou não confere, fica no
# robô, e a linha diz qual e de que tamanho. Nunca sobrescreve o que já está neste Mac.
guardar_sobras() { # guardar_sobras <linhas "<bytes> <nome>">
    local g r pasta bytes nome sufixo destino la aqui
    { read -r g; read -r r; } < <(gravacoes)
    [ -n "$1" ] || return 0
    pasta="$(dirname "$CHROOT$g")"
    while read -r bytes nome; do
        [ -n "$nome" ] || continue
        sufixo="${nome##*.robo-}"
        case "$nome" in
            "$(basename "$g")".robo-*) destino="$OUT/robo-$sufixo.quadros" ;;
            *) destino="$OUT/robo-$sufixo.recebido.txt" ;;
        esac
        while [ -e "$destino" ]; do destino="$destino.outra"; done
        if ! "${A[@]}" pull "$pasta/$nome" "$destino.copiando" >/dev/null 2>&1; then
            rm -f "$destino.copiando"
            diz "AVISO: a gravação antiga $nome ($((bytes / 1024 / 1024)) MB) não foi copiada do robô, e segue lá."
            continue
        fi
        la="$("${A[@]}" shell "$BB sha256sum $pasta/$nome" 2>/dev/null | tr -d '\r' | cut -d' ' -f1)"
        aqui="$(shasum -a 256 "$destino.copiando" | cut -d' ' -f1)"
        if [ -n "$la" ] && [ "$la" = "$aqui" ]; then
            mv "$destino.copiando" "$destino"
            "${A[@]}" shell "$BB rm -f $pasta/$nome" >/dev/null 2>&1 || true
            echo "gravação antiga $nome guardada em $destino"
        else
            rm -f "$destino.copiando"
            diz "AVISO: a gravação antiga $nome ($((bytes / 1024 / 1024)) MB) chegou com a soma diferente, e segue no robô."
        fi
    done <<< "$1"
}

# O disco do robô, antes de parar qualquer coisa: a gravação precisa de duas vezes o limite dela livre (uma inteira ainda cabe
# depois de uma esquecida). Com pouco espaço e gravações esquecidas, elas são guardadas primeiro, à vista. Sem resposta, para.
SOBRAS=""
conferir_disco() { # conferir_disco <limite da gravação em MiB>
    local precisa=$(($1 * 2 * 1024)) livre
    livre="$(espaco_livre_kib)"
    [ -n "$livre" ] || falha "não consegui ler o espaço do disco do robô; nada foi parado nele. Tente de novo"
    SOBRAS="$(sobras_no_robo)"
    if [ "$livre" -lt "$precisa" ] && [ -n "$SOBRAS" ]; then
        passo "Guardando as gravações antigas do robô"
        guardar_sobras "$SOBRAS"
        livre="$(espaco_livre_kib)"
        [ -n "$livre" ] || falha "não consegui ler o espaço do disco do robô; nada foi parado nele. Tente de novo"
        SOBRAS="$(sobras_no_robo)"
    fi
    if [ "$livre" -lt "$precisa" ]; then
        local quantas=0
        if [ -n "$SOBRAS" ]; then quantas="$(printf '%s\n' "$SOBRAS" | wc -l | tr -d ' ')"; fi
        falha "o disco do robô tem $((livre / 1024)) MB livres, e a gravação precisa de $((precisa / 1024)); gravações antigas no robô: $quantas. Nada foi parado nele"
    fi
    if [ -n "$SOBRAS" ]; then diz "AVISO: há gravações antigas no robô ($(printf '%s\n' "$SOBRAS" | wc -l | tr -d ' ')); a cópia delas roda ao fundo."; fi
}

# A cópia ao fundo termina antes de outra ativação ou da volta: o reinício a cortaria no meio. Só espera a cópia, que nasce de
# um `up` desta bancada, e nunca a si mesma nem outro processo que herdou o número (a ponte também tem `rms1d-bench` no nome
# do socket). Com o prazo vencido, segue: a gravação fica no robô, com o nome da ativação dela.
esperar_copia() {
    local pid limite=$((SECONDS + ${S1_ESPERA_COPIA:-120}))
    [ -f "$ESTADO/copia.pid" ] || return 0
    pid="$(cat "$ESTADO/copia.pid")"
    [ "$pid" != "$$" ] || return 0
    while ps -p "$pid" -o command= 2>/dev/null | grep -q "rms1d-bench.sh up"; do
        if [ "$SECONDS" -ge "$limite" ]; then
            echo "a cópia da gravação anterior não terminou em ${S1_ESPERA_COPIA:-120} s; ela fica no robô, com o nome da ativação dela"
            return 0
        fi
        sleep 0.2
    done
}

# O socket do `rms1-mediad` no robô, lido de `[midia]` no arquivo; vazio se o arquivo não tem mídia.
socket_da_midia() {
    python3 -I -c 'import sys,tomllib; c=tomllib.load(open(sys.argv[1],"rb")); print(c["midia"].get("socket","/run/rms1-mediad/media.sock") if "midia" in c else "")' "$TOML"
}

# A página até o `rms1d` do robô: a porta do `adb` e a ponte do socket. Com mídia, o mesmo até o `rms1-mediad`.
ligar_ponte() {
    "${A[@]}" forward "tcp:$TCP" "localfilesystem:$CHROOT/run/robotd.sock" >/dev/null
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    nohup python3 -I scripts/s1/rms1d-ponte.py "$SOCKET" "127.0.0.1:$TCP" >"$ESTADO/ponte.log" 2>&1 </dev/null &
    echo $! > "$ESTADO/ponte.pid"
    local m
    m="$(socket_da_midia)"
    if [ -n "$m" ]; then
        "${A[@]}" forward "tcp:$TCP_MIDIA" "localfilesystem:$CHROOT$m" >/dev/null
        if [ -f "$ESTADO/ponte-midia.pid" ]; then kill "$(cat "$ESTADO/ponte-midia.pid")" 2>/dev/null || true; fi
        nohup python3 -I scripts/s1/rms1d-ponte.py "$SOCKET_MIDIA" "127.0.0.1:$TCP_MIDIA" >"$ESTADO/ponte-midia.log" 2>&1 </dev/null &
        echo $! > "$ESTADO/ponte-midia.pid"
    fi
    sleep 1
}

# A mídia do robô: envia o `rms1-mediad`, o dispositivo do microfone e os sons, e o sobe. A câmera e o microfone já estão
# livres: o `dji_camera` foi parado e conferido antes. Uma falha aqui não impede pilotar: avisa, com o motivo, e segue.
subir_midia() { # subir_midia <socket no robô> <1 se o binário do robô já é este>
    local pasta
    pasta="$(dirname "$1")"
    if [ -z "$2" ]; then "${A[@]}" push "$BIN_MIDIA" "$CHROOT/opt/robot/bin/rms1-mediad" >/dev/null; fi
    "${A[@]}" push deploy/s1/asound.conf "$CHROOT/etc/asound.conf" >/dev/null
    local copia=""
    for par in $SONS; do
        copia="$copia $BB mkdir -p $CHROOT/var/lib/robot/sounds/${par%%:*}; $BB cp -f /system/audio/sound/${par#*:}.wav $CHROOT/var/lib/robot/sounds/${par%%:*}/;"
    done
    "${A[@]}" shell "chmod 755 $CHROOT/opt/robot/bin/rms1-mediad; mkdir -p $CHROOT$pasta; $copia"
    "${A[@]}" shell "($BB setsid $BB nohup $ENTRA /opt/robot/bin/rms1-mediad --config /etc/robot/rms1d.toml </dev/null >/data/rms1-mediad.log 2>&1 &); $BB sleep 3"
    "${A[@]}" shell "$BB pidof rms1-mediad >/dev/null && echo midia_no_ar || { echo MIDIA_MORTA; $BB tail -3 /data/rms1-mediad.log; }" | tr -d '\r' | tee "$OUT/midia.txt"
    if ! grep -q midia_no_ar "$OUT/midia.txt"; then
        diz "AVISO: a mídia do robô não subiu ($(grep -v MIDIA_MORTA "$OUT/midia.txt" | tail -1)). Dá para pilotar; a câmera, o microfone e o som ficam fora."
    fi
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
        cargo s1 -p rms1d -p rms1-mediad
        cargo build -q -p rms1d -p robotctl
    fi
    # Antes de qualquer coisa no robô: o arquivo recusado, com dji_scratch por exemplo, não chega a parar nada.
    local parar
    parar="$(target/debug/rms1d --config "$TOML" --processos-a-parar)" || falha "$TOML recusado; nada foi tocado no robô"
    local limite
    limite="$(target/debug/rms1d --config "$TOML" --limite-da-gravacao)"
    # Sem o binário, a soma vazia daria "igual" à do robô sem binário, e a DJI seria parada antes da falha.
    [ -s "$BIN" ] || falha "não achei $BIN; compile com cargo s1 -p rms1d. Nada foi tocado no robô"
    local midia
    midia="$(socket_da_midia)"
    if [ -n "$midia" ]; then
        [ -s "$BIN_MIDIA" ] || falha "não achei $BIN_MIDIA; compile com cargo s1 -p rms1-mediad. Nada foi tocado no robô"
    fi
    passo "Abrindo o acesso ao robô"
    python3 -I "$ACESSO" abrir --host "$IP"
    confere chroot_ok "sem chroot em $CHROOT; rode scripts/s1/bench.sh exec uma vez" "test -d $CHROOT/bin && echo chroot_ok"
    "${A[@]}" shell "$BB grep -q ' $CHROOT/dev ' /proc/mounts || mount -o bind /dev $CHROOT/dev; $BB grep -q ' $CHROOT/proc ' /proc/mounts || mount -t proc proc $CHROOT/proc; $BB grep -q ' $CHROOT/sys ' /proc/mounts || mount -t sysfs sys $CHROOT/sys; $BB grep -q ' $CHROOT/mnt/dji ' /proc/mounts || { $BB mkdir -p $CHROOT/mnt/dji && mount -o bind /data $CHROOT/mnt/dji; }"
    "${A[@]}" shell "mkdir -p $CHROOT/opt/robot/bin $CHROOT/etc/robot $CHROOT/run $CHROOT/var/lib/robot"
    if [ -n "$limite" ]; then conferir_disco "$limite"; fi
    # O que o robô tem. O busybox dele tem `sha256sum` (lido na cópia completa, `arvores/system/system/xbin/busybox`); o
    # `adb shell` do Android 4.4 termina a linha com `\r`.
    local estado soma_bin soma_toml vivo=""
    estado="$("${A[@]}" shell "echo estado_do_robo; $BB pidof rms1d >/dev/null && echo RMS1D_VIVO; $BB pidof rms1-mediad >/dev/null && echo MIDIA_VIVA; $BB sha256sum $CHROOT/opt/robot/bin/rms1d $CHROOT/etc/robot/rms1d.toml $CHROOT/opt/robot/bin/rms1-mediad 2>/dev/null" | tr -d '\r' || true)"
    echo "$estado"
    case "$estado" in *RMS1D_VIVO*) vivo=1 ;; esac
    soma_bin="$(printf '%s\n' "$estado" | grep '/opt/robot/bin/rms1d$' | cut -d' ' -f1 || true)"
    soma_toml="$(printf '%s\n' "$estado" | grep '/etc/robot/rms1d.toml$' | cut -d' ' -f1 || true)"
    local mesmo_bin="" mesmo_toml=""
    if [ -n "$soma_bin" ] && [ "$soma_bin" = "$(shasum -a 256 "$BIN" | cut -d' ' -f1)" ]; then mesmo_bin=1; fi
    if [ -n "$soma_toml" ] && [ "$soma_toml" = "$(shasum -a 256 "$TOML" | cut -d' ' -f1)" ]; then mesmo_toml=1; fi
    # Com mídia no arquivo, "o mesmo no ar" inclui o `rms1-mediad`: o mesmo binário, e vivo.
    local mesma_midia="" midia_ok=1
    if [ -n "$midia" ]; then
        if [ "$(printf '%s\n' "$estado" | grep '/opt/robot/bin/rms1-mediad$' | cut -d' ' -f1 || true)" = "$(shasum -a 256 "$BIN_MIDIA" | cut -d' ' -f1)" ]; then mesma_midia=1; fi
        case "$estado" in *MIDIA_VIVA*) ;; *) midia_ok="" ;; esac
        [ -n "$mesma_midia" ] || midia_ok=""
    fi
    if [ -n "$vivo" ] && [ -n "$mesmo_bin" ] && [ -n "$mesmo_toml" ] && [ -n "$midia_ok" ]; then
        passo "Conectando ao robô"
        ligar_ponte
        if saude; then
            if [ -n "$SOBRAS" ]; then copiar_ao_fundo ""; fi
            passo
            diz "Robô pronto"
            return
        fi
        echo "o rms1d no robô não respondeu bem: preparando de novo"
    fi
    if [ -n "$vivo" ]; then passo "Atualizando o robô"; else passo "Preparando o robô"; fi
    # Um `rms1d` de antes sai pelo caminho que para o robô, e antes do envio: binário em uso não se sobrescreve, e a trava
    # faria o novo morrer em silêncio, com o velho respondendo no lugar dele.
    # O `rms1-mediad` de antes sai primeiro, por SIGTERM: é assim que ele desliga o fluxo da câmera antes de soltá-la.
    confere sem_midia "o rms1-mediad anterior não saiu; desligue o robô na chave" "$BB pkill -TERM rms1-mediad; $BB sleep 1; $BB pidof rms1-mediad >/dev/null && { $BB sleep 2; $BB pidof rms1-mediad >/dev/null && echo MIDIA_ANTIGA_VIVA || echo sem_midia; } || echo sem_midia"
    confere sem_rms1d "o rms1d anterior não saiu; desligue o robô na chave" "$BB pkill -TERM rms1d; $BB sleep 1; $BB pidof rms1d >/dev/null && echo RMS1D_ANTIGO_VIVO || echo sem_rms1d"
    separar_gravacao "robo-$(date +%Y%m%d-%H%M%S)"
    if [ -z "$mesmo_bin" ]; then "${A[@]}" push "$BIN" "$CHROOT/opt/robot/bin/rms1d" >/dev/null; fi
    "${A[@]}" push "$TOML" "$CHROOT/etc/robot/rms1d.toml" >/dev/null
    "${A[@]}" shell "chmod 755 $CHROOT/opt/robot/bin/rms1d"
    # Os processos da DJI que seguram a serial: parados só se estiverem no ar, e a serial conferida livre sempre.
    # O `dji_camera` sai só pelo `init` (`stop` e `setprop ctl.stop`), nunca por `kill`: morto no meio do fluxo, ele deixou a
    # câmera respondendo ENODEV até reiniciar (2026-10-30). Espera-se até 5 s por ele.
    local lista="" vivos=""
    for p in $parar; do
        if [ "$p" = dji_camera ]; then
            lista="$lista $BB pidof $p >/dev/null && { stop $p; setprop ctl.stop $p; for n in 1 2 3 4 5; do $BB pidof $p >/dev/null || break; $BB sleep 1; done; };"
        else
            lista="$lista $BB pidof $p >/dev/null && { stop $p; setprop ctl.stop $p; kill \$($BB pidof $p) 2>/dev/null; $BB sleep 1; };"
        fi
        vivos="$vivos $BB pidof $p >/dev/null && echo AINDA_NO_AR_$p;"
    done
    local saida
    saida="$("${A[@]}" shell "$lista $vivos for f in /proc/[0-9]*/fd/*; do $BB readlink \$f 2>/dev/null | $BB grep -q ttyS3 && echo SERIAL_PRESA \$f; done; echo parada_conferida" | tee "$OUT/parar.txt")"
    echo "$saida"
    case "$saida" in *parada_conferida*) ;; *) falha "a parada dos processos da DJI não respondeu" ;; esac
    case "$saida" in *AINDA_NO_AR_dji_camera*) falha "o dji_camera não saiu pelo init, e ele não é morto à força: a câmera ficaria travada. Reinicie o robô" ;; esac
    case "$saida" in *AINDA_NO_AR*|*SERIAL_PRESA*) falha "a serial do controlador não ficou livre: dois programas nela é o que este passo evita. Reinicie o robô" ;; esac
    # A mídia antes do `rms1d`: quando ele começa a repassar som, o `rms1-mediad` já está ouvindo.
    if [ -n "$midia" ]; then
        case " $parar " in
            *" dji_camera "*) subir_midia "$midia" "$mesma_midia" ;;
            *) diz "AVISO: o arquivo tem [midia] e não manda parar o dji_camera; a mídia do robô não foi ligada." ;;
        esac
    fi
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
    if [ -f "$ESTADO/ponte-midia.pid" ]; then kill "$(cat "$ESTADO/ponte-midia.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP_MIDIA" >/dev/null 2>&1 || true
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
    # SIGTERM: o `rms1d` para o robô e descarrega a gravação antes de sair; o `rms1-mediad` desliga o fluxo da câmera.
    "${A[@]}" shell "$BB pkill -TERM rms1-mediad; $BB pkill -TERM rms1d; $BB sleep 1" || true
    guardar_gravacao "robo-$(date +%H%M%S)"
    if [ -f "$ESTADO/ponte.pid" ]; then kill "$(cat "$ESTADO/ponte.pid")" 2>/dev/null || true; fi
    if [ -f "$ESTADO/ponte-midia.pid" ]; then kill "$(cat "$ESTADO/ponte-midia.pid")" 2>/dev/null || true; fi
    "${A[@]}" forward --remove "tcp:$TCP_MIDIA" >/dev/null 2>&1 || true
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
