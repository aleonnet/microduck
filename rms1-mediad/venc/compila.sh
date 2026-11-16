#!/bin/sh
# Compila o rms1-venc para o lado Android do S1 (Android 4.4.4, API 19, ARM de 32 bits) com o NDK deste Mac, e confere, sem o
# robô, que o programa só pede ao sistema o que o sistema do robô tem.
#
# O NDK de hoje compila da API 21 para cima, e o robô é 19. Por isso a conferência: cada símbolo que o programa importa tem de
# estar exportado nas bibliotecas do próprio robô, lidas da cópia de 2026-11-01 no laboratório
# (out/2026-11-01-copia-completa/arvores/system/system/lib). Um símbolo que o robô não tem reprova aqui, e não lá.
#
# Uso: sh rms1-mediad/venc/compila.sh        → target/s1-android/rms1-venc, e "RMS1-VENC CONFERE"
# Ambiente: ANDROID_NDK (padrão: o mais novo de ~/Library/Android/sdk/ndk), S1_LAB (padrão ~/Development/RoboMaster-S1).
set -eu
AQUI=$(cd "$(dirname "$0")" && pwd)
RAIZ=$(cd "$AQUI/../.." && pwd)
LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
SISTEMA="$LAB/out/2026-11-01-copia-completa/arvores/system/system/lib"
NDK="${ANDROID_NDK:-$(ls -d "$HOME"/Library/Android/sdk/ndk/* 2>/dev/null | sort | tail -1)}"
[ -d "$NDK" ] || { echo "não achei o Android NDK; aponte ANDROID_NDK"; exit 1; }
[ -f "$SISTEMA/libc.so" ] || { echo "não achei a cópia do sistema do robô em $SISTEMA (o WD_BLACK está montado?)"; exit 1; }
BIN=$(ls -d "$NDK"/toolchains/llvm/prebuilt/*/bin | head -1)
SAIDA="$RAIZ/target/s1-android"
mkdir -p "$SAIDA"
"$BIN/armv7a-linux-androideabi21-clang" -O2 -Wall -Wextra -Werror -fPIE -pie -o "$SAIDA/rms1-venc" "$AQUI/rms1-venc.c" -ldl
# O que o programa importa, contra o que o robô exporta.
"$BIN/llvm-nm" -D --undefined-only "$SAIDA/rms1-venc" | awk '{print $NF}' | sed 's/@.*//' | sort -u > "$SAIDA/importa.txt"
for b in libc.so libdl.so libm.so; do "$BIN/llvm-nm" -D --defined-only "$SISTEMA/$b" | awk '{print $NF}'; done | sort -u > "$SAIDA/robo-exporta.txt"
FALTA=$(comm -23 "$SAIDA/importa.txt" "$SAIDA/robo-exporta.txt")
if [ -n "$FALTA" ]; then echo "o robô não tem o que o programa pede:"; echo "$FALTA"; exit 1; fi
# As bibliotecas que ele pede têm de existir no robô, e o carregador tem de ser o dele.
for b in $("$BIN/llvm-readelf" -d "$SAIDA/rms1-venc" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p'); do
    [ -f "$SISTEMA/$b" ] || { echo "o robô não tem a biblioteca $b"; exit 1; }
done
"$BIN/llvm-readelf" -l "$SAIDA/rms1-venc" | grep -q '/system/bin/linker\]' || { echo "o carregador pedido não é /system/bin/linker"; exit 1; }
# O carregador do Android 4.4 só lê a tabela de símbolos antiga (DT_HASH).
"$BIN/llvm-readelf" -d "$SAIDA/rms1-venc" | grep -q '(HASH)' || { echo "falta a tabela DT_HASH, que o carregador do Android 4.4 exige"; exit 1; }
echo "$(wc -l < "$SAIDA/importa.txt" | tr -d ' ') símbolos importados, todos exportados pelo sistema do robô"
shasum -a 256 "$SAIDA/rms1-venc"
echo "RMS1-VENC CONFERE"
