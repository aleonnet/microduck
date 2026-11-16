#!/usr/bin/env bash
# Os dois passos da prova contra `adb` e acesso falsos: pede só o previsto, e nunca o que derruba o robô.
# O que este teste não pega é o que só o robô responde.
set -euo pipefail
cd "$(dirname "$0")/../.."
T="$(mktemp -d /tmp/rms1b.XXXXXX)"
parar_ponte() { for f in "$T/e/ponte.pid" "$T/e/ponte-midia.pid"; do if [ -f "$f" ]; then kill "$(cat "$f")" 2>/dev/null || true; fi; done; }
# Ao sair, com ou sem falha: as pontes param, a cópia ao fundo acaba, e a pasta deste teste some. O que reprova já foi à tela.
limpar() {
    parar_ponte
    if [ -f "$T/e/copia.pid" ]; then while kill -0 "$(cat "$T/e/copia.pid")" 2>/dev/null; do sleep 0.1; done; fi
    rm -rf "$T"
}
trap limpar EXIT
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
# Conexão caída: o `adb` de verdade sai com 1 quando o aparelho não está conectado (medido), até `abrir` reconectar.
if tem adb_caiu && [ ! -f "$DEFEITO.conectado" ]; then exit 1; fi
case "$*" in
  *estado_do_robo*)
    # O que o robô tem: o `rms1d` no ar e as somas do programa e da configuração.
    if tem ja_no_ar; then echo RMS1D_VIVO; tem midia_caiu || echo MIDIA_VIVA; tem venc_caiu || echo VENC_VIVO; printf '%s  /data/debian/opt/robot/bin/rms1-mediad\r\n' "$SOMA_MIDIA"; printf '%s  /data/debian/opt/robot/bin/rms1d\r\n%s  /data/debian/etc/robot/rms1d.toml\r\n' "$SOMA_BIN" "$SOMA_TOML";
    elif tem outro_no_ar; then echo RMS1D_VIVO; printf '%s  /data/debian/opt/robot/bin/rms1d\r\n%s  /data/debian/etc/robot/rms1d.toml\r\n' 0000 0000;
    elif tem outro_toml; then echo RMS1D_VIVO; printf '%s  /data/debian/opt/robot/bin/rms1d\r\n%s  /data/debian/etc/robot/rms1d.toml\r\n' "$SOMA_BIN" 0000;
    elif tem vivo_sem_somas; then echo RMS1D_VIVO;
    elif tem mesmo_binario; then printf '%s  /data/debian/opt/robot/bin/rms1d\r\n' "$SOMA_BIN"; fi ;;
  *"test -d"*) tem sem_chroot || echo chroot_ok ;;
  # O disco do robô, no formato do `busybox df -k /data` medido no robô em 2026-11-15 (duas linhas, com `\r`): cheio como naquele
  # dia, ou com folga. Guardadas as sobras, sobra espaço.
  *espaco_do_robo*)
    if tem disco_mudo; then :
    elif tem espaco_do_dono; then printf 'espaco_do_robo\r\n/dev/block/platform/comip-mmc.1/by-name/userdata\r\n                       1015704    901024     98296  91%% /data\r\n'
    elif tem pouco_espaco && [ ! -f "$DEFEITO.sobras_guardadas" ]; then printf 'espaco_do_robo\r\n/dev/block/platform/comip-mmc.1/by-name/userdata\r\n                       1015704    997904      1416 100%% /data\r\n'
    else printf 'espaco_do_robo\r\n/dev/block/platform/comip-mmc.1/by-name/userdata\r\n                       1015704    828268    171052  83%% /data\r\n'; fi ;;
  # A pasta das gravações, no formato do `busybox ls -l` pelo `adb shell`, com as cores: uma gravação esquecida, até ser apagada.
  *sobras_do_robo*)
    echo sobras_do_robo
    if tem sobras && [ ! -f "$DEFEITO.sobras_guardadas" ]; then printf -- '-rw-rw-rw-    1 0        0         39428096 Jan  1 01:19 \033[0;0mrms1d-recebido.txt.robo-20261101-010101\033[0m\r\n'; fi
    printf 'drwxrwxrwx    8 0        0             4096 Jan  1 00:02 \033[1;34msounds\033[0m\r\n' ;;
  *"sha256sum /data/debian/var/lib/robot/"*".robo-"*)
    if tem soma_diferente; then printf '0000000000000000000000000000000000000000000000000000000000000000  x\r\n'
    else printf '%s  x\r\n' "$(printf 'gravacao antiga' | shasum -a 256 | cut -d' ' -f1)"; fi ;;
  *"rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101"*) touch "$DEFEITO.sobras_guardadas" ;;
  *"pkill -TERM rms1-mediad"*sem_midia*) if tem midia_antiga; then echo MIDIA_ANTIGA_VIVA; else echo sem_midia; fi ;;
  *midia_no_ar*) if tem midia_morta; then echo MIDIA_MORTA; echo "rms1-mediad: mixer: o valor não pegou em MIC2 Volume"; else echo midia_no_ar; fi ;;
  *"pkill -TERM rms1d"*sem_rms1d*) if tem rms1d_antigo; then echo RMS1D_ANTIGO_VIVO; else echo sem_rms1d; fi ;;
  *parada_conferida*) tem dji_vivo && echo AINDA_NO_AR_dji_hdvt_uav; tem camera_viva && echo AINDA_NO_AR_dji_camera; tem serial_presa && echo SERIAL_PRESA /proc/9/fd/3; echo parada_conferida ;;
  *venc_no_ar*) if tem venc_morto; then echo VENC_MORTO; echo "não abri libOMX.hantro.H1.video.encoder.so"; else echo venc_no_ar; fi ;;
  *"pidof rms1d"*) if tem rms1d_morto; then echo RMS1D_MORTO; else echo rms1d_no_ar; fi ;;
  *"pidof dji_hdvt_uav"*) echo dji_voltou ;;
  *marca_gravada*) if tem sem_marca; then exit 0; fi; touch "$DEFEITO.marca"; echo marca_gravada ;;
  *marca_viva*) if [ -f "$DEFEITO.uma_falha" ] && [ -f "$DEFEITO.reiniciou" ]; then rm -f "$DEFEITO.uma_falha"; exit 1; fi; if [ -f "$DEFEITO.marca" ]; then echo marca_viva; else echo marca_sumiu; fi ;;
  # Só a cópia ao fundo da ativação, de nome `.robo-`, fica lenta: a da própria volta, rápida, não cobre a falta da espera.
  *" pull "*".robo-"*) if tem copia_lenta; then sleep 4; fi; if tem pull_falha; then exit 1; fi
    # A cópia chega: o último argumento é o destino neste Mac.
    for d; do :; done; printf 'gravacao antiga' > "$d" ;;
  *"shell reboot"*) touch "$DEFEITO.reiniciou"; rm -f "$DEFEITO.conectado"; tem nao_reinicia || rm -f "$DEFEITO.marca" ;;
esac
exit 0
SHIM
chmod +x "$T/bin/adb"
cat > "$T/acesso.py" <<'SHIM'
import os, sys
print("acesso", sys.argv[1:])
defeito = os.environ["DEFEITO"]
if "sem_robo" in open(defeito).read().split():
    print("Lab is busy; leaving the existing program untouched", file=sys.stderr)
    sys.exit(1)
open(defeito + ".conectado", "w").close()
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
# A cópia da gravação anterior roda ao fundo, logo depois de separada: espera ela acabar antes de olhar ou zerar o registro.
esperar_copia() { for f in "$T/e/copia.pid"; do [ -f "$f" ] && while kill -0 "$(cat "$f")" 2>/dev/null; do sleep 0.1; done; done; return 0; }
# reprova <nome> <palavra na mensagem> <o que não pode ter sido pedido>: `up` tem de parar, com a mensagem.
reprova() {
    : > "$ADB_LOG"
    if scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1; then echo "SUBIU COM DEFEITO: $1"; falhou=1; return; fi
    if grep -qF -- "$2" "$T/up.txt"; then echo "parou em $1: $2"; else echo "PAROU SEM DIZER $2 em $1:"; tail -5 "$T/up.txt"; falhou=1; fi
    if [ -n "$3" ] && grep -qF -- "$3" "$ADB_LOG"; then echo "SEGUIU ADIANTE em $1: $3"; falhou=1; fi
}

cargo build -q -p rms1d -p robotctl
export SOMA_BIN="$(shasum -a 256 target/armv7-unknown-linux-gnueabihf/release/rms1d | cut -d' ' -f1)"
export SOMA_TOML="$(shasum -a 256 deploy/s1/rms1d.toml | cut -d' ' -f1)"
export SOMA_MIDIA="$(shasum -a 256 target/armv7-unknown-linux-gnueabihf/release/rms1-mediad | cut -d' ' -f1)"
# O codificador de hardware: um arquivo qualquer no lugar do binário, que só o Android NDK compila.
export S1_BIN_VENC="$T/rms1-venc"; printf 'rms1-venc de teste' > "$S1_BIN_VENC"

echo "--- o caminho certo"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up falhou:"; tail -20 "$T/up.txt"; exit 1; }
grep -q "Robô pronto" "$T/up.txt" || { echo "up não terminou:"; tail -20 "$T/up.txt"; exit 1; }
pediu "stop dji_hdvt_uav"
pediu "stop dji_monitor"
# A mídia: o `dji_camera` sai só pelo `init`, nunca por `kill`; o `rms1-mediad` sobe depois dele e antes do `rms1d`.
pediu "stop dji_camera; setprop ctl.stop dji_camera"
nunca 'kill $(/system/xbin/busybox pidof dji_camera)'
antes "stop dji_camera" "/opt/robot/bin/rms1-mediad --config /etc/robot/rms1d.toml"
antes "/opt/robot/bin/rms1-mediad --config" "/opt/robot/bin/rms1d --config"
antes "pkill -TERM rms1-mediad" "push target/armv7-unknown-linux-gnueabihf/release/rms1-mediad"
# O codificador de hardware: o de antes sai depois do rms1-mediad, que é o cliente dele; o novo vai para o lado Android do robô
# e sobe antes do rms1-mediad, no socket que o arquivo dá, visto de fora do chroot.
antes "pkill -TERM rms1-mediad" "pkill -TERM rms1-venc"
pediu "push $S1_BIN_VENC /data/rms1-venc"
antes "/data/rms1-venc servir /data/debian/run/rms1-venc/venc.sock" "/opt/robot/bin/rms1-mediad --config /etc/robot/rms1d.toml"
pediu "push deploy/s1/asound.conf /data/debian/etc/asound.conf"
pediu "cp -f /system/audio/sound/power_on.wav /data/debian/var/lib/robot/sounds/greet/"
pediu "forward tcp:9011 localfilesystem:/data/debian/run/rms1-mediad/media.sock"
if grep -q "AVISO" "$T/up.txt"; then echo "AVISO NO CAMINHO CERTO:"; grep AVISO "$T/up.txt"; falhou=1; fi
# A mira da DJI: o `/data` dela dentro do chroot, ligado só se ainda não estiver, antes de o `rms1d` subir.
pediu "grep -q ' /data/debian/mnt/dji ' /proc/mounts || {"
pediu "mkdir -p /data/debian/mnt/dji && mount -o bind /data /data/debian/mnt/dji; }"
antes "mount -o bind /data /data/debian/mnt/dji" "/opt/robot/bin/rms1d --config /etc/robot/rms1d.toml"
pediu "/opt/robot/bin/rms1d --config /etc/robot/rms1d.toml --socket /run/robotd.sock"
pediu "forward tcp:9010 localfilesystem:/data/debian/run/robotd.sock"
pediu "push deploy/s1/rms1d.toml /data/debian/etc/robot/rms1d.toml"
antes "pkill -TERM rms1d" "push target/armv7-unknown-linux-gnueabihf/release/rms1d"
antes "stop dji_monitor" "/opt/robot/bin/rms1d --config"
antes "pkill -TERM rms1d" "mv -f /data/debian/var/lib/robot/rms1d-quadros.ndjson"
antes "mv -f /data/debian/var/lib/robot/rms1d-quadros.ndjson" "/opt/robot/bin/rms1d --config"
grep "stop dji_monitor" "$ADB_LOG" | grep -q "stop dji_monitor.*stop dji_hdvt_uav" || { echo "O MONITOR NÃO VEIO PRIMEIRO"; falhou=1; }
# A gravação anterior muda para um nome desta ativação e é copiada ao fundo, logo; só é apagada no robô depois de chegar.
esperar_copia
antes "mv -f /data/debian/var/lib/robot/rms1d-quadros.ndjson /data/debian/var/lib/robot/rms1d-quadros.ndjson.robo-" "pull /data/debian/var/lib/robot/rms1d-quadros.ndjson.robo-"
antes "pull /data/debian/var/lib/robot/rms1d-recebido.txt.robo-" "rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-"
grep -q "Preparando o robô" "$T/up.txt" && echo "nada nosso no robô: prepara" || { echo "NADA NOSSO, SEM PREPARAR:"; tail -5 "$T/up.txt"; falhou=1; }
grep -q "Robô pronto" "$T/up.txt" && echo "termina em Robô pronto" || { echo "NÃO DISSE ROBÔ PRONTO"; falhou=1; }
pediu "push target/armv7-unknown-linux-gnueabihf/release/rms1d"
# Cada passo termina com o tempo dele: tantas linhas de tempo quanto passos, e cada uma logo depois do seu passo.
python3 -I - "$T/up.txt" <<'PY' || falhou=1
import re, sys
linhas = [l[len("[rms1d-bench] "):].rstrip("\n") for l in open(sys.argv[1], encoding="utf-8") if l.startswith("[rms1d-bench] ")]
passos = [l for l in linhas if not l.startswith(("TEMPO ", "AVISO", "Robô pronto"))]
tempos = [l for l in linhas if re.fullmatch(r"TEMPO \d+ .+", l)]
# Cada passo tem o seu tempo, com o mesmo texto, antes do passo seguinte.
fechados = [re.fullmatch(r"TEMPO \d+ (.+)", l).group(1) for l in tempos]
seguido = fechados == passos
print(("tempo por passo: %d passos, %d tempos" % (len(passos), len(tempos))) if len(passos) == len(tempos) and seguido
      else "PASSO SEM TEMPO: %r" % linhas)
sys.exit(0 if len(passos) == len(tempos) and seguido else 1)
PY

echo "--- a mídia que não sobe avisa, com o motivo, e não impede pilotar"
echo midia_morta > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com a mídia morta falhou:"; tail -5 "$T/up.txt"; falhou=1; }
esperar_copia
grep -q "AVISO: a mídia do robô não subiu (rms1-mediad: mixer: o valor não pegou em MIC2 Volume)" "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt" && echo "mídia morta: avisa e segue" || { echo "MÍDIA MORTA, SEM AVISO:"; tail -5 "$T/up.txt"; falhou=1; }
: > "$DEFEITO"

echo "--- o dji_camera que não sai pelo init para tudo, sem kill e sem subir a mídia"
echo camera_viva > "$DEFEITO"
reprova "dji_camera vivo" "o dji_camera não saiu pelo init" "/opt/robot/bin/rms1-mediad --config"
esperar_copia
: > "$DEFEITO"

echo "--- o codificador de hardware que não sobe, e o que não foi compilado: avisa, e a ativação segue"
echo venc_morto > "$DEFEITO"
: > "$ADB_LOG"
if scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 && grep -q "AVISO: o codificador de hardware não subiu (não abri libOMX.hantro.H1.video.encoder.so). O quadro vai cru à estação." "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt"; then echo "codificador morto: avisa e segue"; else echo "CODIFICADOR MORTO, E NÃO AVISOU OU NÃO SEGUIU:"; tail -4 "$T/up.txt"; falhou=1; fi
esperar_copia
: > "$DEFEITO"
: > "$ADB_LOG"
if S1_BIN_VENC="$T/nao-existe" scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 && grep -q "AVISO: o arquivo pede o codificador de hardware, e não achei $T/nao-existe" "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt" && ! grep -q "rms1-venc servir" "$ADB_LOG"; then echo "codificador não compilado: avisa, não tenta subir, e segue"; else echo "CODIFICADOR NÃO COMPILADO, E NÃO AVISOU OU TENTOU SUBIR:"; tail -4 "$T/up.txt"; falhou=1; fi
esperar_copia

echo "--- o rms1-mediad anterior que não sai"
echo midia_antiga > "$DEFEITO"
reprova "rms1-mediad antigo" "o rms1-mediad anterior não saiu" "stop dji_camera"
: > "$DEFEITO"

echo "--- o mesmo binário não vai de novo"
echo mesmo_binario > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com o mesmo binário falhou:"; tail -5 "$T/up.txt"; falhou=1; }
if grep -qF "push target/armv7-unknown-linux-gnueabihf/release/rms1d" "$ADB_LOG"; then echo "ENVIOU O MESMO BINÁRIO"; falhou=1; else echo "mesmo binário: não enviou"; fi
pediu "push deploy/s1/rms1d.toml /data/debian/etc/robot/rms1d.toml"
esperar_copia
: > "$DEFEITO"

echo "--- o mesmo rms1d no ar, saudável: só conecta"
echo ja_no_ar > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com o rms1d no ar falhou:"; tail -5 "$T/up.txt"; falhou=1; }
grep -q "Conectando ao robô" "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt" && echo "no ar e igual: conecta" || { echo "NO AR E IGUAL, NÃO CONECTOU:"; tail -5 "$T/up.txt"; falhou=1; }
for proibido in "pkill -TERM rms1d" "pkill -TERM rms1-mediad" "pkill -TERM rms1-venc" "rms1-venc servir" "push target" "stop dji_monitor" "stop dji_camera" "/opt/robot/bin/rms1d --config" "/opt/robot/bin/rms1-mediad --config"; do
    if grep -qF -- "$proibido" "$ADB_LOG"; then echo "NO AR E IGUAL, MAS PEDIU: $proibido"; falhou=1; fi
done
pediu "forward tcp:9010 localfilesystem:/data/debian/run/robotd.sock"
: > "$DEFEITO"

echo "--- o mesmo no ar, com o codificador de hardware caído: não só conecta, sobe o codificador de novo"
printf 'ja_no_ar\nvenc_caiu\n' > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com o codificador caído falhou:"; tail -5 "$T/up.txt"; falhou=1; }
esperar_copia
if grep -q "Atualizando o robô" "$T/up.txt" && ! grep -q "Conectando ao robô" "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt"; then echo "codificador caído: atualiza"; else echo "CODIFICADOR CAÍDO, E SÓ CONECTOU:"; tail -5 "$T/up.txt"; falhou=1; fi
pediu "/data/rms1-venc servir /data/debian/run/rms1-venc/venc.sock"
: > "$DEFEITO"

# atualiza <defeito> <envia o binário: sim|nao>: outro `rms1d` no ar, ou o mesmo com outra configuração, ou sem as somas: atualiza,
# e nunca só conecta. Uma soma vazia ou diferente nunca vale como igual.
atualiza() {
    echo "$1" > "$DEFEITO"
    : > "$ADB_LOG"
    scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up em $1 falhou:"; tail -5 "$T/up.txt"; falhou=1; }
    esperar_copia
    if grep -q "Atualizando o robô" "$T/up.txt" && ! grep -q "Conectando ao robô" "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt"; then echo "$1: atualiza"; else echo "$1: NÃO ATUALIZOU:"; tail -5 "$T/up.txt"; falhou=1; fi
    pediu "pkill -TERM rms1d"
    pediu "push deploy/s1/rms1d.toml /data/debian/etc/robot/rms1d.toml"
    if grep -qF "push target/armv7-unknown-linux-gnueabihf/release/rms1d" "$ADB_LOG"; then enviou=sim; else enviou=nao; fi
    [ "$enviou" = "$2" ] && echo "$1: binário enviado $enviou" || { echo "$1: BINÁRIO ENVIADO $enviou, ESPERADO $2"; falhou=1; }
    : > "$DEFEITO"
}
echo "--- outro rms1d no ar: atualiza"
atualiza outro_no_ar sim
echo "--- o mesmo rms1d no ar com outra configuração: atualiza"
atualiza outro_toml nao
echo "--- o rms1d no ar sem as somas: atualiza"
atualiza vivo_sem_somas sim

echo "--- o caso do dono em 2026-11-15: 95 MB livres, sem gravação esquecida. Uma gravação inteira (64 MiB) cabe: a ativação segue"
echo espaco_do_dono > "$DEFEITO"
: > "$ADB_LOG"
if scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 && grep -q "Robô pronto" "$T/up.txt" && ! grep -q "MB livres" "$T/up.txt"; then echo "95 MB livres: a ativação seguiu até o Robô pronto"; else echo "95 MB LIVRES, E A ATIVAÇÃO PAROU:"; tail -3 "$T/up.txt"; falhou=1; fi
esperar_copia

echo "--- adendo 2: o disco do robô com pouco espaço, sem gravação esquecida: para antes de tocar na DJI"
echo pouco_espaco > "$DEFEITO"
reprova "pouco espaço" "o disco do robô tem 1 MB livres, e a gravação precisa de 64" "stop dji_monitor"
if grep -qE "pkill|push " "$ADB_LOG"; then echo "POUCO ESPAÇO, E MEXEU NO ROBÔ:"; grep -E "pkill|push " "$ADB_LOG" | head -3; falhou=1; else echo "pouco espaço: nada parado nem enviado"; fi
echo "--- adendo 2: o robô não diz o espaço do disco: para, sem subir às cegas"
echo disco_mudo > "$DEFEITO"
reprova "disco mudo" "não consegui ler o espaço do disco do robô" "stop dji_monitor"
echo "--- adendo 2: uma gravação esquecida no robô, com espaço: copiada ao fundo, conferida pela soma, e só então apagada"
echo sobras > "$DEFEITO"; rm -f "$DEFEITO.sobras_guardadas" "$T/out/robo-20261101-010101.recebido.txt"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com sobras falhou:"; tail -5 "$T/up.txt"; falhou=1; }
esperar_copia
antes "pull /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101" "sha256sum /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101"
antes "sha256sum /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101" "rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101"
if [ -f "$T/out/robo-20261101-010101.recebido.txt" ] && grep -q "AVISO: há gravações antigas no robô (1)" "$T/up.txt"; then echo "sobra: guardada neste Mac, e a ativação avisou"; else echo "SOBRA NÃO GUARDADA, OU SEM AVISO:"; tail -5 "$T/up.txt"; falhou=1; fi
echo "--- adendo 2: gravação esquecida e pouco espaço: guardada à vista antes de conferir o disco, e a ativação segue"
printf 'sobras\npouco_espaco\n' > "$DEFEITO"; rm -f "$DEFEITO.sobras_guardadas" "$T/out/robo-20261101-010101.recebido.txt"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com sobras e pouco espaço falhou:"; tail -5 "$T/up.txt"; falhou=1; }
esperar_copia
grep -q "Guardando as gravações antigas do robô" "$T/up.txt" && grep -q "Robô pronto" "$T/up.txt" && echo "sobras com pouco espaço: guarda à vista e segue" || { echo "SOBRAS COM POUCO ESPAÇO, SEM O PASSO:"; tail -5 "$T/up.txt"; falhou=1; }
antes "rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101" "stop dji_monitor"
echo "--- adendo 2: a cópia chega com a soma diferente: avisa pelo nome, e não apaga no robô"
printf 'sobras\npouco_espaco\nsoma_diferente\n' > "$DEFEITO"; rm -f "$DEFEITO.sobras_guardadas" "$T/out/robo-20261101-010101.recebido.txt"
reprova "soma diferente" "MB livres" "stop dji_monitor"
grep -q "AVISO: a gravação antiga rms1d-recebido.txt.robo-20261101-010101 (37 MB) chegou com a soma diferente" "$T/up.txt" && echo "soma diferente: avisa pelo nome e pelo tamanho" || { echo "SOMA DIFERENTE, SEM AVISO:"; tail -5 "$T/up.txt"; falhou=1; }
if grep -qF "rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-20261101-010101" "$ADB_LOG"; then echo "APAGOU NO ROBÔ COM A SOMA DIFERENTE"; falhou=1; else echo "soma diferente: a gravação fica no robô"; fi
[ ! -e "$T/out/robo-20261101-010101.recebido.txt" ] && [ ! -e "$T/out/robo-20261101-010101.recebido.txt.copiando" ] && echo "soma diferente: nada fica neste Mac" || { echo "SOMA DIFERENTE, E FICOU ARQUIVO NO MAC"; falhou=1; }
: > "$DEFEITO"; rm -f "$DEFEITO.sobras_guardadas"

echo "--- as gravações ficam fora dos repositórios"
rm -rf "$T/xdg"
env -u S1_OUT XDG_STATE_HOME="$T/xdg" S1_LAB="$T/lab" scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up sem S1_OUT falhou:"; tail -5 "$T/up.txt"; falhou=1; }
esperar_copia
if [ -f "$T/xdg/rms1/gravacoes/$(date +%Y-%m-%d)/up.txt" ] && [ ! -e "$T/lab/out" ]; then echo "gravações em \$XDG_STATE_HOME/rms1/gravacoes/<data>/, nada no laboratório"; else echo "GRAVAÇÕES FORA DO LUGAR:"; find "$T/xdg" "$T/lab" 2>/dev/null | head; falhou=1; fi

echo "--- o que tem de parar a subida"
printf '[dji]\nparar = ["dji_scratch"]\n' > "$T/ruim.toml"
: > "$ADB_LOG"
if S1_TOML="$T/ruim.toml" scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1; then echo "SUBIU COM dji_scratch"; falhou=1; fi
grep -qF "recusado" "$T/up.txt" || { echo "NÃO DISSE RECUSADO:"; tail -5 "$T/up.txt"; falhou=1; }
if [ -s "$ADB_LOG" ]; then echo "TOCOU O ROBÔ COM O ARQUIVO RECUSADO:"; cat "$ADB_LOG"; falhou=1; else echo "arquivo recusado: nada foi pedido ao robô"; fi

: > "$ADB_LOG"
if S1_BIN="$T/nao-existe" scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1; then echo "SUBIU SEM O BINÁRIO"; falhou=1; fi
grep -qF "não achei $T/nao-existe" "$T/up.txt" || { echo "SEM O BINÁRIO, SEM DIZER:"; tail -3 "$T/up.txt"; falhou=1; }
if [ -s "$ADB_LOG" ]; then echo "TOCOU O ROBÔ SEM O BINÁRIO:"; head -3 "$ADB_LOG"; falhou=1; else echo "sem o binário: nada foi pedido ao robô"; fi
: > "$T/vazio"
: > "$ADB_LOG"
if S1_BIN="$T/vazio" scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1; then echo "SUBIU COM O BINÁRIO VAZIO"; falhou=1; fi
grep -qF "não achei $T/vazio" "$T/up.txt" || { echo "BINÁRIO VAZIO, SEM DIZER:"; tail -3 "$T/up.txt"; falhou=1; }
if [ -s "$ADB_LOG" ]; then echo "TOCOU O ROBÔ COM O BINÁRIO VAZIO:"; head -3 "$ADB_LOG"; falhou=1; else echo "binário vazio: nada foi pedido ao robô"; fi

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
esperar_copia
pediu "pull /data/debian/var/lib/robot/rms1d-quadros.ndjson.robo-"
echo serial_muda > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "SERIAL MUDA PAROU A SUBIDA:"; tail -5 "$T/up.txt"; falhou=1; }
if grep -q "AVISO: nada chegou do controlador" "$T/up.txt"; then echo "serial muda: avisa e segue"; else echo "SERIAL MUDA SEM AVISO"; falhou=1; fi
: > "$DEFEITO"

echo "--- a volta espera a cópia da ativação antes de reiniciar"
echo copia_lenta > "$DEFEITO"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "up com a cópia lenta falhou:"; tail -5 "$T/up.txt"; falhou=1; }
scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 || { echo "volta com a cópia lenta falhou:"; tail -5 "$T/volta.txt"; falhou=1; }
antes "rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-" "shell reboot"
rm -f "$DEFEITO.reiniciou"
echo "--- a ativação seguinte espera a cópia da anterior antes de tocar o robô"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "primeira subida com a cópia lenta falhou:"; tail -5 "$T/up.txt"; falhou=1; }
scripts/s1/rms1d-bench.sh up > "$T/up2.txt" 2>&1 || { echo "segunda subida com a cópia lenta falhou:"; tail -5 "$T/up2.txt"; falhou=1; }
esperar_copia
apagou="$(grep -nF "rm -f /data/debian/var/lib/robot/rms1d-recebido.txt.robo-" "$ADB_LOG" | head -1 | cut -d: -f1 || true)"
segunda="$(grep -nF "estado_do_robo" "$ADB_LOG" | sed -n 2p | cut -d: -f1 || true)"
if [ -n "$apagou" ] && [ -n "$segunda" ] && [ "$apagou" -lt "$segunda" ]; then echo "segunda subida: depois da cópia da primeira"; else echo "SEGUNDA SUBIDA ANTES DA CÓPIA DA PRIMEIRA: $apagou, $segunda"; falhou=1; fi
: > "$DEFEITO"

echo "--- a cópia que falha não apaga a gravação no robô, e cada ativação usa um nome seu"
echo pull_falha > "$DEFEITO"
: > "$ADB_LOG"
for n in 1 2; do scripts/s1/rms1d-bench.sh up > "$T/up.txt" 2>&1 || { echo "subida $n com o pull falhando parou:"; tail -5 "$T/up.txt"; falhou=1; }; done
esperar_copia
if grep -qF "rm -f /data/debian/var/lib/robot/rms1d-quadros.ndjson.robo-" "$ADB_LOG"; then echo "APAGOU NO ROBÔ SEM A CÓPIA TER CHEGADO"; falhou=1; else echo "pull falhou: a gravação fica no robô"; fi
nomes="$(grep -oE "mv -f /data/debian/var/lib/robot/rms1d-quadros.ndjson /data/debian/var/lib/robot/rms1d-quadros.ndjson[.][^ ]+" "$ADB_LOG" | sort -u | wc -l | tr -d ' ')"
if [ "$nomes" = 2 ]; then echo "duas ativações: dois nomes"; else echo "DUAS ATIVAÇÕES, $nomes NOME(S)"; falhou=1; fi
grep -qE "mv -f /data/debian/var/lib/robot/rms1d-quadros.ndjson /data/debian/var/lib/robot/rms1d-quadros.ndjson[.]robo-[0-9]{8}-[0-9]{6}" "$ADB_LOG" && echo "o nome leva a data" || { echo "O NOME NÃO LEVA A DATA"; falhou=1; }
[ ! -e "$T/e/copia.pid" ] && echo "a cópia apaga o seu número ao terminar" || { echo "O NÚMERO DA CÓPIA FICOU"; falhou=1; }
: > "$DEFEITO"

echo "--- a espera não espera outro processo que herdou o número da cópia"
bash -c 'exec -a rms1d-bench-alheio sleep 30' &
alheio=$!
echo "$alheio" > "$T/e/copia.pid"
inicio=$SECONDS
S1_ESPERA_COPIA=20 scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 || true
if [ $((SECONDS - inicio)) -lt 15 ]; then echo "número herdado: não esperou"; else echo "ESPEROU UM PROCESSO ALHEIO"; falhou=1; fi
kill "$alheio" 2>/dev/null || true
rm -f "$T/e/copia.pid" "$DEFEITO.reiniciou"

echo "--- a volta"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 || { echo "volta falhou:"; tail -20 "$T/volta.txt"; exit 1; }
pediu "pkill -TERM rms1d"
pediu "pkill -TERM rms1-mediad"
pediu "pkill -TERM rms1-venc"
pediu "forward --remove tcp:9011"
pediu "shell reboot"
pediu "forward --remove tcp:9010"
pediu "pidof dji_hdvt_uav"
antes "marca_gravada" "shell reboot"
echo nao_reinicia > "$DEFEITO"
rm -f "$DEFEITO.reiniciou"
if scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1; then echo "DISSE QUE VOLTOU SEM REINICIAR"; falhou=1; fi
grep -qF "a DJI não voltou" "$T/volta.txt" && echo "sem reiniciar: diz que não voltou" || { echo "SEM REINICIAR, CALADO:"; tail -3 "$T/volta.txt"; falhou=1; }
: > "$DEFEITO"

echo "--- a volta com o adb caído"
echo adb_caiu > "$DEFEITO"
rm -f "$DEFEITO.reiniciou" "$DEFEITO.conectado"
: > "$ADB_LOG"
scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 || { echo "VOLTA COM O ADB CAÍDO FALHOU:"; tail -5 "$T/volta.txt"; falhou=1; }
pediu "shell reboot"
: > "$DEFEITO"
rm -f "$DEFEITO.reiniciou"
touch "$DEFEITO.uma_falha"
scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1 && echo "uma falha do adb na espera: tenta de novo" || { echo "UMA FALHA DO ADB NA ESPERA DERRUBOU A VOLTA:"; tail -5 "$T/volta.txt"; falhou=1; }
rm -f "$DEFEITO.uma_falha" "$DEFEITO.reiniciou" "$DEFEITO.conectado"
printf 'adb_caiu\nsem_robo\n' > "$DEFEITO"
: > "$ADB_LOG"
if scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1; then echo "DISSE QUE VOLTOU SEM ROBÔ"; falhou=1; fi
grep -qF "PAROU: não consegui abrir o acesso ao robô" "$T/volta.txt" && grep "PAROU" "$T/volta.txt" | grep -qF "Lab is busy" && echo "acesso falhou: diz o motivo real" || { echo "ACESSO FALHOU SEM O MOTIVO:"; tail -3 "$T/volta.txt"; falhou=1; }
if grep -qF "shell reboot" "$ADB_LOG"; then echo "REINICIOU SEM ACESSO"; falhou=1; fi
: > "$DEFEITO"
echo sem_marca > "$DEFEITO"
rm -f "$DEFEITO.marca" "$DEFEITO.reiniciou"
: > "$ADB_LOG"
if scripts/s1/rms1d-bench.sh volta > "$T/volta.txt" 2>&1; then echo "VOLTOU SEM PROVA DO REINÍCIO"; falhou=1; fi
grep -qF "não consegui marcar" "$T/volta.txt" && echo "sem marca: não reinicia às cegas" || { echo "SEM MARCA, SEM DIZER:"; tail -3 "$T/volta.txt"; falhou=1; }
if grep -qF "shell reboot" "$ADB_LOG"; then echo "REINICIOU SEM MARCA"; falhou=1; fi
: > "$DEFEITO"
nunca "dji_scratch"
nunca "dji_sys"
nunca "dji_network"
# O `dji_camera` só sai pelo `init`: nunca por `kill`, nem `pkill`, nem `killall`.
nunca 'kill $(/system/xbin/busybox pidof dji_camera)'
nunca "pkill dji_camera"
nunca "pkill -TERM dji_camera"
nunca "pkill -9 dji_camera"
nunca "killall dji_camera"
nunca "0x48"
nunca "stop dji_vision"
nunca "dd if="
nunca "remount"
nunca ".anterior"
[ "$falhou" = 0 ] || { echo "BANCADA FALSA REPROVOU"; exit 1; }
echo "BANCADA FALSA OK"
