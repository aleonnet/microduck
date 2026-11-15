#!/bin/sh
# O quadro que o S1 manda à estação, provado contra o programa de 2026-11-13: a redução do `rms1-mediad` (NV12 1280x720 para
# RGB 320x180) seguida da moldura da estação (`tools/hailo/servico/moldura.c`, no laboratório) tem de dar, byte a byte, o
# mesmo que o `tools/hailo/prepara_s1.c`, o programa que preparou os quadros da medição de 13/11.
#
#   sh scripts/test-rms1-mediad-lab.sh             MOLDURA IGUAL
#   sh scripts/test-rms1-mediad-lab.sh --plantar   troca o cinza da moldura numa cópia e exige a diferença: DEFEITO DETECTADO
#
# Só no Mac: compila os dois programas em C com o `cc` daqui e usa um quadro gravado da câmera do robô. Não toca o robô.
# Ambiente: S1_LAB (padrão ~/Development/RoboMaster-S1).
set -eu
cd "$(dirname "$0")/.."
LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
QUADRO="$LAB/out/2026-10-31-poc2/camera/frame_000.nv12"
[ -f "$QUADRO" ] || { echo "PAROU: não achei $QUADRO. Aponte o laboratório com S1_LAB=<pasta>" >&2; exit 1; }
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

cargo build -q -p rms1-mediad --bin rms1-mediad-prova
cc -O2 -Wall -Wextra -Werror "$LAB/tools/hailo/prepara_s1.c" -o "$T/prepara_s1"
MOLDURA="$LAB/tools/hailo/servico/moldura.c"
if [ "${1:-}" = "--plantar" ]; then
    # Na cópia, o cinza do fundo vira preto: as 70 linhas de cima e de baixo deixam de bater.
    sed 's/#define MOLDURA_CINZA 114/#define MOLDURA_CINZA 0/' "$LAB/tools/hailo/servico/moldura.h" > "$T/moldura.h"
    cmp -s "$T/moldura.h" "$LAB/tools/hailo/servico/moldura.h" && { echo "PAROU: o defeito não foi plantado" >&2; exit 1; }
    cp "$MOLDURA" "$T/moldura.c"
    MOLDURA="$T/moldura.c"
fi
cc -O2 -Wall -Wextra -Werror -DMOLDURA_PROVA "$MOLDURA" -o "$T/moldura_prova"

"$T/prepara_s1" "$QUADRO" 2 "$T/de_13_11.rgb" > /dev/null
target/debug/rms1-mediad-prova reducao "$QUADRO" 320 180 "$T/reduzido.rgb"
"$T/moldura_prova" "$T/reduzido.rgb" 320 180 320 "$T/de_agora.rgb"

[ "$(wc -c < "$T/reduzido.rgb" | tr -d ' ')" = 172800 ] || { echo "PAROU: o quadro reduzido não tem 172800 bytes" >&2; exit 1; }
[ "$(wc -c < "$T/de_agora.rgb" | tr -d ' ')" = 307200 ] || { echo "PAROU: o quadro emoldurado não tem 307200 bytes" >&2; exit 1; }
if cmp -s "$T/de_13_11.rgb" "$T/de_agora.rgb"; then
    [ "${1:-}" = "--plantar" ] && { echo "DEFEITO NAO DETECTADO"; exit 1; }
    echo "MOLDURA IGUAL: $(shasum -a 256 "$T/de_agora.rgb" | cut -c1-16), 307200 bytes"
else
    [ "${1:-}" = "--plantar" ] && { echo "DEFEITO DETECTADO: $(cmp "$T/de_13_11.rgb" "$T/de_agora.rgb" 2>&1 | head -1 | sed "s|$T/||g")"; exit 0; }
    echo "MOLDURA DIFERE: $(cmp "$T/de_13_11.rgb" "$T/de_agora.rgb" 2>&1 | head -1 | sed "s|$T/||g")"
    exit 1
fi
