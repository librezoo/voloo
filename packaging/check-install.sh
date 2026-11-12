#!/bin/sh
# Installs the built npm packages and wheel into throw-away locations and exercises them
# with simulated devices only (VOLOO_DEMO, separate VOLOO_HOME, USB scanning off).
# Usage: sh packaging/check-install.sh   (after packaging/npm/build.mjs and the maturin build)
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
PY=${PYTHON:-python3}

check() { # $1 = label, $2 = voloo command
  label=$1; voloo=$2
  home="$WORK/home-$label"; mkdir -p "$home"
  run() { VOLOO_HOME="$home" VOLOO_DEMO=2 VOLOO_SCAN_USB=0 $voloo "$@"; }
  echo "== $label: $($voloo --version)"
  run --json ls | "$PY" -c 'import json,sys; d=json.load(sys.stdin); assert [x["port"] for x in d]==["sim://1","sim://2"], d; print("ls ok:", len(d), "demo devices")'
  run --json on bench-a | "$PY" -c 'import json,sys; d=json.load(sys.stdin); assert d["outcome"]=="completed", d; print("on ok")'
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"check","version":"1"}}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | run mcp | "$PY" -c 'import json,sys; r=[json.loads(l) for l in sys.stdin]; assert r[0]["result"]["serverInfo"]["name"]=="voloo"; print("mcp ok:", len(r[1]["result"]["tools"]), "tools")'
  run --json report | "$PY" -c 'import json,sys; print("report ok:", json.load(sys.stdin)["path"].split("/")[-1])'
  pid=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["pid"])' "$home/daemon.json")
  echo "background process: $(ps -o command= -p "$pid")"
  run daemon stop >/dev/null
}

echo "## npm (isolated prefix)"
mkdir -p "$WORK/npm" && cd "$WORK/npm" && npm init -y >/dev/null
npm install --omit=optional --no-audit --no-fund "$ROOT"/dist/npm/librezoo-voloo-*.tgz >/dev/null
check npm "$WORK/npm/node_modules/.bin/voloo"

echo "## pip (fresh virtualenv)"
"$PY" -m venv "$WORK/venv"
"$WORK/venv/bin/pip" install --quiet --no-index "$ROOT"/dist/pypi/voloo-*.whl
check pip "$WORK/venv/bin/voloo"

if command -v uvx >/dev/null 2>&1; then
  echo "## uvx (ephemeral)"
  wheel=$(ls "$ROOT"/dist/pypi/voloo-*.whl | head -1)
  check uvx "uvx --quiet --from $wheel voloo"
fi
echo "all install checks passed"
