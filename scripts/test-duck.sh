#!/bin/sh
# What `scripts/duck` promises, checked with fakes: nothing starts and no browser opens.
#
#   sh scripts/test-duck.sh            # prints DUCK OK
#   sh scripts/test-duck.sh --plantar  # breaks each rule in a copy; prints DEFEITO DETECTADO
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
COMMAND="$HERE/duck"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
REPO="$WORK/parent/microduck"
RL="$WORK/parent/microduck_rl"
mkdir -p "$WORK/bin" "$REPO/scripts" "$RL"
LOG="$WORK/log"
COUNT="$WORK/count"
export LOG COUNT

cat > "$REPO/scripts/duck-sim" <<'STUB'
#!/bin/sh
echo "sim $* cameras=${DUCK_SIM_CAMERAS:-} viewer=${DUCK_SIM_VIEWER:-} rl=${DUCK_SIM_RL:-}" >> "$LOG"
STUB
# Fails CURL_FAILS times before it answers, which is a console still coming up.
cat > "$WORK/bin/curl" <<'STUB'
#!/bin/sh
echo curl >> "$LOG"
n=$(($(cat "$COUNT") + 1))
echo "$n" > "$COUNT"
[ "$n" -gt "$CURL_FAILS" ]
STUB
for opener in open xdg-open; do
    cat > "$WORK/bin/$opener" <<'STUB'
#!/bin/sh
echo "page $1" >> "$LOG"
STUB
done
printf '#!/bin/sh\n' > "$WORK/bin/sleep"
printf '#!/bin/sh\necho 192.0.2.7\n' > "$WORK/bin/ipconfig"
chmod +x "$REPO/scripts/duck-sim" "$WORK/bin/"*

# One run of the copy, with a clean log and none of the caller's own settings.
run() {
    : > "$LOG"
    echo 0 > "$COUNT"
    env -u DUCK_SIM_RL -u DUCK_SIM_CAMERAS -u DUCK_SIM_VIEWER PATH="$WORK/bin:$PATH" "$@" \
        >/dev/null 2>&1
}

# Runs every check against the command it is given. Prints the first failure and returns non-zero.
check() {
    cp "$1" "$REPO/scripts/duck"
    duck="$REPO/scripts/duck"
    defaults="cameras=a viewer=0 rl=$RL"

    run CURL_FAILS=2 sh "$duck" || { echo "FALHOU AO LIGAR"; return 1; }
    [ "$(sed -n 1p "$LOG")" = "sim up $defaults" ] || { echo "INICIO: $(sed -n 1p "$LOG")"; return 1; }
    [ "$(sed 1d "$LOG" | tr '\n' ' ')" = "curl curl curl page http://127.0.0.1:8080 " ] ||
        { echo "ORDEM: $(sed 1d "$LOG" | tr '\n' ' ')"; return 1; }

    run CURL_FAILS=0 DUCK_SIM_CAMERAS=c DUCK_SIM_VIEWER=1 DUCK_SIM_RL=/elsewhere sh "$duck" ||
        { echo "FALHOU COM VALOR DEFINIDO"; return 1; }
    [ "$(sed -n 1p "$LOG")" = "sim up cameras=c viewer=1 rl=/elsewhere" ] ||
        { echo "VALOR DEFINIDO: $(sed -n 1p "$LOG")"; return 1; }

    run CURL_FAILS=0 sh "$duck" down || { echo "FALHOU COM ARGUMENTO"; return 1; }
    [ "$(cat "$LOG")" = "sim down $defaults" ] ||
        { echo "ARGUMENTO: $(tr '\n' ' ' < "$LOG")"; return 1; }

    if run CURL_FAILS=999 sh "$duck"; then
        echo "SEM CONSOLE: saiu com sucesso"
        return 1
    fi
    if grep -q '^page' "$LOG"; then
        echo "SEM CONSOLE: abriu a pagina"
        return 1
    fi
    return 0
}

if [ "${1:-}" != --plantar ]; then
    check "$COMMAND" || { echo "DUCK REPROVADO"; exit 1; }
    echo "DUCK OK"
    exit 0
fi

# Each rule broken in a copy, with the failure it has to produce.
falhou=0
plantar() {
    nome="$1"; de="$2"; para="$3"; esperado="$4"
    copia="$WORK/copia"
    grep -qF -- "$de" "$COMMAND" || { echo "$nome: trecho nao encontrado"; falhou=1; return; }
    awk -v de="$de" -v para="$para" \
        '{ i = index($0, de); if (i) $0 = substr($0, 1, i - 1) para substr($0, i + length(de)); print }' \
        "$COMMAND" > "$copia"
    saida="$(check "$copia" 2>&1 || true)"
    case "$saida" in
        "$esperado"*) echo "$nome -> $saida" ;;
        *) echo "$nome -> NAO REPROVOU como devia: ${saida:-passou}"; falhou=1 ;;
    esac
}
plantar "sem espera" 'until curl -fsS -o /dev/null --max-time 1 "$PAGE" 2>/dev/null; do' 'until true; do' "ORDEM:"
plantar "sem camera" '${DUCK_SIM_CAMERAS:-a}' '${DUCK_SIM_CAMERAS:-}' "INICIO:"
plantar "com janela" '${DUCK_SIM_VIEWER:-0}' '${DUCK_SIM_VIEWER:-1}' "INICIO:"
plantar "ignora o valor definido" 'DUCK_SIM_VIEWER="${DUCK_SIM_VIEWER:-0}"' 'DUCK_SIM_VIEWER=0' "VALOR DEFINIDO:"
plantar "argumento abre a pagina" '|| exec "$REPO/scripts/duck-sim" "$@"' '|| "$REPO/scripts/duck-sim" "$@"' "ARGUMENTO:"
plantar "desiste em silencio" 'at $PAGE" >&2; exit 1; }' 'at $PAGE" >&2; exit 0; }' "SEM CONSOLE:"

if [ "$falhou" = 0 ]; then
    echo "DEFEITO DETECTADO"
    exit 0
fi
echo "DEFEITO NAO DETECTADO"
exit 1
