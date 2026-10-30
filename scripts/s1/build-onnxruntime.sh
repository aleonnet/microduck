#!/usr/bin/env bash
# Build libonnxruntime.so for the S1 (32-bit ARM, hard-float, glibc) — Microsoft publishes no
# armv7 binary, and `duck-detect` and `robotd` reach ONNX Runtime through `ort` with
# `load-dynamic`, which wants exactly this .so.
#
# The recipe is the project's own "Cross compiling on Linux" (onnxruntime.ai/docs/build/
# inferencing.html#arm): a CMake toolchain file naming the cross compiler, and
# `-Donnxruntime_ENABLE_CPUINFO=OFF`. Cross rather than native because the same page says a
# 32-bit compiler "might not have enough memory to run the build" and the build "needs more than
# 2GB memory". The host is an arm64 Debian container, which this Mac runs natively, with Debian's
# own `g++-arm-linux-gnueabihf`; cmake comes from pip because the page asks for 3.28+ and bookworm
# ships 3.25. The tag is the workspace's `[workspace.metadata.onnxruntime] target`.
#
# ARM32 is "experimental" in ONNX Runtime's own words. This script's job is to reach the end —
# a .so with its sha256, or the first error verbatim in <out>/build.log — never to pretend.
#
# Usage: scripts/s1/build-onnxruntime.sh [out-dir] [tag]
set -euo pipefail

OUT="${1:-$HOME/Development/RoboMaster-S1/out/onnxruntime}"
TAG="${2:-v$(python3 -c 'import re,sys;t=open(sys.argv[1]).read();print(re.search(r"\[workspace\.metadata\.onnxruntime\][^\[]*target\s*=\s*\"([^\"]+)\"",t,re.S).group(1))' "$(dirname "$0")/../../Cargo.toml")}"
NAME="s1-ort-build-$$"
JOBS="${JOBS:-4}"

if ! docker version >/dev/null 2>&1; then
    echo "docker is not running; open -a Docker" >&2
    exit 1
fi
mkdir -p "$OUT"
echo "[ort] building onnxruntime $TAG for armv7hf in an arm64 bookworm container (jobs=$JOBS)"
echo "[ort] log: $OUT/build.log"

# `--compile-no-warning-as-error` (a cmake flag) is what ONNX Runtime's own build.py hands to cmake
# when asked `--compile_no_warning_as_error` (tools/ci_build/build.py, use_dev_mode). Without it
# the armv7 build dies on -Werror=restrict, a GCC 12 false positive in libstdc++ char_traits on
# 32-bit targets. -Wno-psabi silences a hundred "parameter passing changed in GCC 7.1" notes.
set +e
docker run --platform linux/arm64 --name "$NAME" -e TAG="$TAG" -e JOBS="$JOBS" debian:bookworm sh -c '
set -e
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
    git python3 python3-pip python3-venv build-essential g++-arm-linux-gnueabihf gcc-arm-linux-gnueabihf \
    ninja-build ca-certificates >/dev/null
python3 -m venv /venv && /venv/bin/pip install -q "cmake>=3.28"
export PATH=/venv/bin:$PATH
cmake --version | head -1
git clone --depth 1 --branch "$TAG" --recursive https://github.com/microsoft/onnxruntime.git /src
cat > /tool.cmake <<TC
SET(CMAKE_SYSTEM_NAME Linux)
SET(CMAKE_SYSTEM_VERSION 1)
SET(CMAKE_SYSTEM_PROCESSOR armv7l)
SET(CMAKE_C_COMPILER arm-linux-gnueabihf-gcc)
SET(CMAKE_CXX_COMPILER arm-linux-gnueabihf-g++)
SET(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
SET(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
SET(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
SET(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
TC
mkdir -p /build && cd /build
cmake /src/cmake -G Ninja --compile-no-warning-as-error \
    -DCMAKE_TOOLCHAIN_FILE=/tool.cmake \
    -DCMAKE_CXX_FLAGS=-Wno-psabi -DCMAKE_C_FLAGS=-Wno-psabi \
    -DCMAKE_BUILD_TYPE=Release \
    -Donnxruntime_BUILD_SHARED_LIB=ON \
    -Donnxruntime_BUILD_UNIT_TESTS=OFF \
    -Donnxruntime_ENABLE_CPUINFO=OFF \
    -Donnxruntime_ENABLE_PYTHON=OFF \
    -DPython_EXECUTABLE=/venv/bin/python3
ninja -j"$JOBS" onnxruntime
ls -la /build/libonnxruntime.so*
' > "$OUT/build.log" 2>&1
STATUS=$?
set -e
if [ "$STATUS" -eq 0 ]; then
    docker cp "$NAME:/build/." "$OUT/build" >/dev/null 2>&1 || true
    cp "$OUT"/build/libonnxruntime.so* "$OUT/" 2>/dev/null || true
    ( cd "$OUT" && shasum -a 256 libonnxruntime.so* > libonnxruntime.sha256 )
    echo "[ort] built: $(ls "$OUT"/libonnxruntime.so* | tr '\n' ' ')"
    cat "$OUT/libonnxruntime.sha256"
else
    echo "[ort] FAILED (exit $STATUS); first error:" >&2
    grep -n -m 5 -i "error" "$OUT/build.log" >&2 || tail -20 "$OUT/build.log" >&2
fi
docker rm "$NAME" >/dev/null 2>&1 || true
exit "$STATUS"
