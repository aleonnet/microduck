#!/bin/sh
# `scripts/rms1` como ele é usado: num terminal qualquer, sem o Python deste Mac, e com cada falha de subida plantada. Cada falha
# tem de aparecer na tela, com o que fazer, sem deixar nada no ar.
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
T="$(mktemp -d /tmp/rms1c.XXXXXX)"
P_PAGINA=18765 P_WS=18766 P_TCP=19000 P_QUADROS=19100
falhou=0
passou() { echo "PASSOU $*"; }
falha() { echo "REPROVOU $*"; falhou=1; }
escutando() { lsof -nP -iTCP:$P_PAGINA -iTCP:$P_WS -iTCP:$P_TCP -iTCP:$P_QUADROS -sTCP:LISTEN 2>/dev/null | grep -c LISTEN; }
[ "$(escutando)" = 0 ] || { echo "portas de teste ocupadas"; exit 2; }
cargo build --quiet -p rms1d || exit 2

UV="$(command -v uv)"
CARGO="$(command -v cargo)"
# Ambiente limpo: só o sistema, o uv e o cargo. Nada do perfil, nada do pyenv.
mkdir -p "$T/limpo"
ln -s "$UV" "$T/limpo/uv"
ln -s "$CARGO" "$T/limpo/cargo"
LIMPO="$T/limpo:/usr/bin:/bin:/usr/sbin:/sbin"
comando() { # comando <arquivo de saída> <PATH> [VAR=valor...]
    saida="$1"; caminho="$2"; shift 2
    # Pelo Python do sistema, para o Ctrl-C chegar como num terminal: grupo de processos próprio, com SIGINT no padrão.
    env -i HOME="$HOME" PATH="$caminho" RMS1_SEM_ABRIR=1 RMS1_ESTADO="$T/estado" RMS1_PORTA_PAGINA=$P_PAGINA RMS1_PORTA_WS=$P_WS \
        RMS1_PORTA_TCP=$P_TCP RMS1_PORTA_QUADROS=$P_QUADROS S1_LAB="$LAB" "$@" /usr/bin/python3 -I -c '
import os, signal, sys
signal.signal(signal.SIGINT, signal.SIG_DFL)
os.setsid()
os.execv("/bin/sh", ["sh", sys.argv[1]])' "$REPO/scripts/rms1" > "$saida" 2>&1 &
    PID=$!
}
ate_no_ar() { # ate_no_ar <arquivo>
    n=0
    until grep -q "no ar:" "$1"; do
        n=$((n + 1))
        kill -0 "$PID" 2>/dev/null || return 1
        [ "$n" -lt 3000 ] || return 1
        sleep 0.1
    done
}
# Espera o comando terminar, com limite: comando que não nota a falha é reprovado aqui, em vez de travar o teste. Saída 124 é o limite.
espera_fim() { # espera_fim <segundos>
    n=0
    while kill -0 "$PID" 2>/dev/null; do
        n=$((n + 1))
        if [ "$n" -gt $(($1 * 10)) ]; then
            kill "$PID" 2>/dev/null
            wait "$PID" 2>/dev/null
            return 124
        fi
        sleep 0.1
    done
    wait "$PID"
}
sem_nada() { # sem_nada <nome>: nem porta, nem processo
    sleep 1
    if [ "$(escutando)" = 0 ] && [ -z "$(pgrep -f "$T/estado" || true)" ]; then passou "$1: não sobrou porta nem processo"
    else falha "$1: sobrou algo: $(escutando) portas, processos $(pgrep -f "$T/estado" | tr '\n' ' ')"; fi
}
recusa_depressa() { # recusa_depressa <nome> <palavra> <PATH> [VAR=valor...]
    nome="$1"; palavra="$2"; caminho="$3"; shift 3
    t0="$(/usr/bin/python3 -c 'import time; print(time.time())')"
    comando "$T/r.txt" "$caminho" "$@"
    wait "$PID"; rc=$?
    dt="$(/usr/bin/python3 -c "import time; print(round(time.time() - $t0, 2))")"
    if [ "$rc" != 0 ] && grep -q "$palavra" "$T/r.txt" && /usr/bin/python3 -c "import sys; sys.exit(0 if $dt < 2 else 1)"; then
        passou "$nome: em $dt s, $(grep PAROU "$T/r.txt")"
    else
        falha "$nome: saída $rc, $dt s: $(cat "$T/r.txt")"
    fi
    [ "$(escutando)" = 0 ] || falha "$nome: subiu alguma coisa"
}

echo "--- o que falta, antes de subir qualquer coisa"
mkdir -p "$T/so-cargo" "$T/so-uv"
ln -s "$CARGO" "$T/so-cargo/cargo"
ln -s "$UV" "$T/so-uv/uv"
recusa_depressa "sem uv" "falta o uv" "$T/so-cargo:/usr/bin:/bin:/usr/sbin:/sbin"
recusa_depressa "sem cargo" "falta o cargo" "$T/so-uv:/usr/bin:/bin:/usr/sbin:/sbin"
recusa_depressa "sem laboratório" "não achei o laboratório" "$LIMPO" S1_LAB=/nao/existe
python3 -I -c '
import socket, sys, time
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(); time.sleep(float(sys.argv[2]))' $P_TCP 4 &
OCUPA=$!
sleep 0.5
t0="$(/usr/bin/python3 -c 'import time; print(time.time())')"
comando "$T/r.txt" "$LIMPO"
wait "$PID"; rc=$?
dt="$(/usr/bin/python3 -c "import time; print(round(time.time() - $t0, 2))")"
if [ "$rc" != 0 ] && grep -q "a porta $P_TCP está ocupada" "$T/r.txt" && /usr/bin/python3 -c "import sys; sys.exit(0 if $dt < 2 else 1)"; then
    passou "porta ocupada: em $dt s, $(grep PAROU "$T/r.txt")"
else
    falha "porta ocupada: saída $rc, $dt s: $(cat "$T/r.txt")"
fi
kill "$OCUPA" 2>/dev/null; wait "$OCUPA" 2>/dev/null

echo "--- sobe de ambiente limpo, com um python3 quebrado na frente"
mkdir -p "$T/quebrado"
printf '#!/bin/sh\necho "$0 $* | pai: $(ps -o command= -p $PPID)" >> "%s/usou-python"\nexit 1\n' "$T" > "$T/quebrado/python3"
chmod +x "$T/quebrado/python3"
comando "$T/sobe.txt" "$T/quebrado:$LIMPO"
if ate_no_ar "$T/sobe.txt"; then passou "sobe sem o Python do terminal: $(grep 'no ar:' "$T/sobe.txt")"; else falha "não subiu: $(cat "$T/sobe.txt")"; fi
if [ -e "$T/usou-python" ]; then falha "usou o python3 do terminal: $(cat "$T/usou-python")"; else passou "não usou o python3 do terminal"; fi
grep -q "compilando" "$T/sobe.txt" && passou "diz que está compilando" || falha "compilou calado"
[ "$(sed -n 1p "$T/sobe.txt" | grep -c compilando)" = 1 ] && passou "diz antes de começar" || falha "a primeira linha não é o aviso: $(sed -n 1p "$T/sobe.txt")"
curl -fs "http://127.0.0.1:$P_PAGINA/robo_s1.json" | grep -q raio_roda && passou "a página serve a tabela do robô" || falha "sem a tabela"
curl -fs -o /dev/null "http://127.0.0.1:$P_PAGINA/viewer/" && passou "a página responde" || falha "a página não responde"
# O Ctrl-C de verdade vai ao grupo inteiro: comando, uv, serviço e rms1d.
kill -INT "-$PID"
espera_fim 10; rc=$?
[ "$rc" = 0 ] && passou "Ctrl-C sai sem erro" || falha "Ctrl-C saiu com $rc: $(tail -3 "$T/sobe.txt")"
grep -q "encerrado" "$T/sobe.txt" && passou "Ctrl-C diz que encerrou" || falha "Ctrl-C calado"
sem_nada "Ctrl-C"

echo "--- scripts/rms1 down, de outro terminal"
comando "$T/sobe.txt" "$LIMPO"
ate_no_ar "$T/sobe.txt" || falha "não subiu para o down"
env -i HOME="$HOME" PATH="$LIMPO" RMS1_ESTADO="$T/estado" sh "$REPO/scripts/rms1" down > "$T/down.txt" 2>&1
espera_fim 10 || true
sem_nada "down"

echo "--- scripts/rms1 down enquanto compila"
mkdir -p "$T/cargo-lento"
printf '#!/bin/sh\nexec sleep 30\n' > "$T/cargo-lento/cargo"
chmod +x "$T/cargo-lento/cargo"
comando "$T/sobe.txt" "$T/cargo-lento:$T/so-uv:/usr/bin:/bin:/usr/sbin:/sbin"
n=0
until grep -q compilando "$T/sobe.txt"; do n=$((n + 1)); [ "$n" -lt 50 ] || break; sleep 0.1; done
sleep 0.3
env -i HOME="$HOME" PATH="$LIMPO" RMS1_ESTADO="$T/estado" sh "$REPO/scripts/rms1" down > "$T/down.txt" 2>&1
if espera_fim 5; then passou "down durante a compilação encerra"; else falha "down durante a compilação não encerrou: $(cat "$T/sobe.txt")"; fi
if pgrep -f "sleep 30" >/dev/null 2>&1 && [ -n "$(pgrep -f "$T/cargo-lento" || true)" ]; then falha "a compilação ficou"; fi
sem_nada "down durante a compilação"

echo "--- o navegador não abre, e há proxy no terminal"
mkdir -p "$T/sem-open"
printf '#!/bin/sh\nexit 1\n' > "$T/sem-open/open"
chmod +x "$T/sem-open/open"
comando "$T/sobe.txt" "$T/sem-open:$LIMPO" RMS1_SEM_ABRIR= http_proxy=http://127.0.0.1:9 all_proxy=http://127.0.0.1:9
if ate_no_ar "$T/sobe.txt"; then passou "sobe com proxy no terminal"; else falha "não subiu com proxy: $(cat "$T/sobe.txt")"; fi
n=0
until grep -q "não consegui abrir o navegador" "$T/sobe.txt" || [ "$n" -gt 30 ]; do n=$((n + 1)); sleep 0.1; done
grep -q "não consegui abrir o navegador" "$T/sobe.txt" && passou "diz que não abriu o navegador" || falha "open falhou calado"
sleep 0.5
if kill -0 "$PID" 2>/dev/null; then passou "open falhando não derruba o comando"; else falha "open falhando derrubou o comando, e o serviço ficou sem dono"; fi
kill -INT "-$PID" 2>/dev/null
espera_fim 10 || true
sem_nada "open falhando"

echo "--- o serviço morre depois de subir"
comando "$T/sobe.txt" "$LIMPO"
ate_no_ar "$T/sobe.txt" || falha "não subiu para morrer"
# Uma ponte até o robô no ar, como depois de uma ativação: morrer não pode deixá-la sem dono.
mkdir -p "$T/estado/bancada"
ln -sf /bin/sleep "$T/rms1d-ponte"
"$T/rms1d-ponte" 300 &
PONTE_FALSA=$!
echo "$PONTE_FALSA" > "$T/estado/bancada/ponte.pid"
SERVICO="$(pgrep -f "s1_servico.py.*$T/estado" | tail -1)"
t0="$(/usr/bin/python3 -c 'import time; print(time.time())')"
kill -9 "$SERVICO"
espera_fim 3; rc=$?
dt="$(/usr/bin/python3 -c "import time; print(round(time.time() - $t0, 2))")"
if [ "$rc" != 0 ] && grep -q "PAROU" "$T/sobe.txt" && /usr/bin/python3 -c "import sys; sys.exit(0 if $dt < 2 else 1)"; then
    passou "morreu depois: em $dt s, $(grep PAROU "$T/sobe.txt")"
else
    falha "morreu depois: saída $rc, $dt s: $(tail -3 "$T/sobe.txt")"
fi
if kill -0 "$PONTE_FALSA" 2>/dev/null; then falha "a ponte até o robô ficou órfã: $PONTE_FALSA"; kill "$PONTE_FALSA" 2>/dev/null; else passou "a ponte até o robô não ficou órfã"; fi
: > "$T/estado/bancada/robo_tocado"
RMS1D_FILHO="$(pgrep -f "$T/estado/rms1d.toml" || true)"
if [ -n "$RMS1D_FILHO" ]; then falha "o rms1d ficou órfão: $RMS1D_FILHO"; kill "$RMS1D_FILHO" 2>/dev/null; fi
sem_nada "morreu depois"

echo "--- o serviço morre ao subir: o erro real aparece"
mkdir -p "$T/lab-ruim/tools/simulador"
printf 'websockets==17.4\n' > "$T/lab-ruim/tools/requirements.txt"
printf 'raise SystemExit("defeito plantado: o servico nao sobe")\n' > "$T/lab-ruim/tools/simulador/s1_servico.py"
comando "$T/r.txt" "$LIMPO" S1_LAB="$T/lab-ruim"
espera_fim 30; rc=$?
if [ "$rc" != 0 ] && grep -q "defeito plantado" "$T/r.txt" && grep -q "PAROU" "$T/r.txt"; then
    passou "morreu ao subir: mostrou o erro real"
else
    falha "morreu ao subir: saída $rc: $(cat "$T/r.txt")"
fi
sem_nada "morreu ao subir"

[ "$falhou" = 0 ] || { echo "RMS1 REPROVOU"; exit 1; }
echo "RMS1 OK"
