#!/bin/sh
# What `scripts/macos/aplay` promises, checked with a fake player so nothing makes a sound.
#
#   sh scripts/test-macos-aplay.sh            # prints APLAY OK
#   sh scripts/test-macos-aplay.sh --plantar  # breaks each rule in a copy; prints DEFEITO DETECTADO
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
ADAPTER="$HERE/macos/aplay"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin"

# One stub for both players: it records how it was called, then stays alive when asked to, so the
# kill below has something to kill.
for player in afplay gst-launch-1.0; do
    cat > "$WORK/bin/$player" <<'STUB'
#!/bin/sh
printf '%s %s\n' "$(basename "$0")" "$*" > "$STUB_LOG"
if [ -n "${STUB_HOLD:-}" ]; then
    echo "$$" > "$STUB_HOLD"
    exec sleep 30
fi
STUB
    chmod +x "$WORK/bin/$player"
done

# Runs every check against the adapter it is given. Prints the first failure and returns non-zero.
check() {
    adapter="$1"
    log="$WORK/log"

    : > "$log"
    PATH="$WORK/bin:$PATH" STUB_LOG="$log" sh "$adapter" -q -D default /tmp/quack.wav
    [ "$(cat "$log")" = "afplay /tmp/quack.wav" ] || { echo "ARQUIVO: $(cat "$log")"; return 1; }

    : > "$log"
    PATH="$WORK/bin:$PATH" STUB_LOG="$log" sh "$adapter" -q -D default -t raw -f S16_LE -c 1 \
        -r 44100 --buffer-time=40000 --period-time=10000
    case "$(cat "$log")" in
        gst-launch-1.0*"fdsrc fd=0"*"sample-rate=44100 num-channels=1"*osxaudiosink) ;;
        *) echo "CRU: $(cat "$log")"; return 1 ;;
    esac

    : > "$log"
    if PATH="$WORK/bin:$PATH" STUB_LOG="$log" sh "$adapter" --bogus /tmp/quack.wav 2>/dev/null; then
        echo "OPCAO DESCONHECIDA ACEITA"; return 1
    fi
    [ ! -s "$log" ] || { echo "OPCAO DESCONHECIDA TOCOU: $(cat "$log")"; return 1; }

    # The kill `robotd` does to a sound it is replacing. With `exec` the pid it holds *is* the
    # player, so the player is gone; without it the pid is a shell and the player plays on.
    hold="$WORK/hold"
    rm -f "$hold"
    PATH="$WORK/bin:$PATH" STUB_LOG="$log" STUB_HOLD="$hold" sh "$adapter" /tmp/quack.wav &
    started=$!
    tries=0
    until [ -s "$hold" ]; do
        tries=$((tries + 1))
        [ "$tries" -lt 50 ] || { echo "TOCADOR NAO SUBIU"; return 1; }
        sleep 0.1
    done
    player="$(cat "$hold")"
    kill -KILL "$started" 2>/dev/null || true
    wait "$started" 2>/dev/null || true
    sleep 0.2
    if kill -0 "$player" 2>/dev/null; then
        kill -KILL "$player" 2>/dev/null || true
        echo "TOCADOR SOBREVIVEU"
        return 1
    fi
    return 0
}

if [ "${1:-}" != --plantar ]; then
    check "$ADAPTER" || { echo "APLAY REPROVADO"; exit 1; }
    echo "APLAY OK"
    exit 0
fi

# Each rule broken in a copy, with the failure it has to produce.
falhou=0
plantar() {
    nome="$1"; de="$2"; para="$3"; esperado="$4"
    copia="$WORK/copia"
    grep -q -- "$de" "$ADAPTER" || { echo "$nome: trecho nao encontrado"; falhou=1; return; }
    sed "s|$de|$para|" "$ADAPTER" > "$copia"
    saida="$(check "$copia" 2>&1 || true)"
    case "$saida" in
        "$esperado"*) echo "$nome -> $saida" ;;
        *) echo "$nome -> NAO REPROVOU como devia: ${saida:-passou}"; falhou=1 ;;
    esac
}
plantar "sem exec" 'exec afplay "\$file"' 'afplay "$file"' "TOCADOR SOBREVIVEU"
plantar "taxa fixa" 'sample-rate="\$rate"' 'sample-rate=48000' "CRU:"
plantar "opcao desconhecida ignorada" '-\*) die "unsupported option: \$1" ;;' '-*) shift ;;' "OPCAO DESCONHECIDA"
plantar "outro tocador" 'exec afplay' 'exec gst-launch-1.0' "ARQUIVO:"

if [ "$falhou" = 0 ]; then
    echo "DEFEITO DETECTADO"
    exit 0
fi
echo "DEFEITO NAO DETECTADO"
exit 1
