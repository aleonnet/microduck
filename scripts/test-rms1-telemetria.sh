#!/bin/sh
# O decodificador de telemetria e o cálculo próprio contra as gravações do robô. Elas ficam no laboratório, que é privado,
# e este repositório é público: por isso não estão no `cargo test` comum. Sem o laboratório, reprova, em vez de passar sem
# medir nada.
set -eu
REPO="$(cd "$(dirname "$0")/.." && pwd)"
S1_LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
export S1_LAB
for g in out/2026-11-02-rms1d-telemetria/s2.strace out/2026-11-03-firmware-controlador/ctrl_fora.strace out/2026-11-03-firmware-controlador/rms1d_fora.strace out/2026-11-03-firmware-controlador/sombra/recebido.txt out/2026-11-03-firmware-controlador/levantado/recebido.txt; do
  [ -f "$S1_LAB/$g" ] || { echo "REPROVOU: não achei $g em $S1_LAB"; exit 1; }
done
cd "$REPO"
cargo test --quiet -p rms1-proto --test gravacoes_lab -- --ignored
cargo test --quiet -p rms1-control --test estimativa_lab -- --ignored
echo "RMS1 TELEMETRIA OK"
