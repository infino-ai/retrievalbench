#!/usr/bin/env bash
# Build the exact FAISS C++ source bundled by faiss-sys 0.7.0.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="$ROOT/Cargo.toml"
BUILD_DIR="$ROOT/target/faiss-build"
INSTALL_DIR="$ROOT/target/faiss"
ENV_FILE="$ROOT/target/faiss-env.sh"

cargo fetch --manifest-path "$MANIFEST"

FAISS_SRC="$(
  python3 - "${CARGO_HOME:-$HOME/.cargo}" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1]) / "registry" / "src"
matches = sorted(root.glob("*/faiss-sys-0.7.0/faiss"))
if len(matches) != 1:
    raise SystemExit(f"expected one bundled FAISS source, found {len(matches)} under {root}")
print(matches[0])
PY
)"

if [ ! -f "$INSTALL_DIR/lib/libfaiss_c.dylib" ] &&
   [ ! -f "$INSTALL_DIR/lib/libfaiss_c.so" ]; then
  cmake -S "$FAISS_SRC" -B "$BUILD_DIR" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR" \
    -DBUILD_SHARED_LIBS=ON \
    -DFAISS_ENABLE_C_API=ON \
    -DFAISS_ENABLE_GPU=OFF \
    -DFAISS_ENABLE_PYTHON=OFF \
    -DBUILD_TESTING=OFF
  cmake --build "$BUILD_DIR" --parallel
  cmake --install "$BUILD_DIR"
fi

mkdir -p "$(dirname "$ENV_FILE")"
cat >"$ENV_FILE" <<EOF
export LIBRARY_PATH="$INSTALL_DIR/lib:$INSTALL_DIR/lib64:\${LIBRARY_PATH:-}"
export DYLD_FALLBACK_LIBRARY_PATH="$INSTALL_DIR/lib:$INSTALL_DIR/lib64:\${DYLD_FALLBACK_LIBRARY_PATH:-}"
export LD_LIBRARY_PATH="$INSTALL_DIR/lib:$INSTALL_DIR/lib64:\${LD_LIBRARY_PATH:-}"
EOF

echo "$ENV_FILE"
