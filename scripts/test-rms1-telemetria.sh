#!/bin/sh
# O decodificador de telemetria contra as gravações do robô. Elas ficam no laboratório, que é privado, e este repositório é
# público: por isso não estão no `cargo test` comum. Sem o laboratório, reprova, em vez de passar sem medir nada.
set -eu
REPO="$(cd "$(dirname "$0")/.." && pwd)"
S1_LAB="${S1_LAB:-$HOME/Development/RoboMaster-S1}"
export S1_LAB
[ -f "$S1_LAB/out/2026-11-02-rms1d-telemetria/s2.strace" ] || { echo "REPROVOU: não achei as gravações em $S1_LAB"; exit 1; }
cd "$REPO"
cargo test --quiet -p rms1-proto --test gravacoes_lab -- --ignored
echo "RMS1 TELEMETRIA OK"
