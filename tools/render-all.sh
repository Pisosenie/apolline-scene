#!/usr/bin/env bash
# Draws one small frame of every project in a folder that prepares natively and prints the
# ones whose drawing fails, a shader that doesn't compile for one. A check that changes to
# shader preparation didn't break scenes that drew before.
#   tools/render-all.sh <workshop folder> <assets folder>
set -euo pipefail
root=${1:?workshop folder}
assets=${2:?assets folder}
cargo build --release -q --example view --example blockers
blocked=$(mktemp)
shot=$(mktemp -d)
trap 'rm -rf "$blocked" "$shot"' EXIT
./target/release/examples/blockers "$root" "$assets" | cut -d' ' -f1 >"$blocked"
drawn=0 failed=0
for project in "$root"/*/; do
  id=$(basename "$project")
  [ -f "$project/scene.pkg" ] || [ -f "$project/scene.json" ] || continue
  grep -qx "$id" "$blocked" && continue
  drawn=$((drawn + 1))
  if ! out=$(./target/release/examples/view --time 2 --shot-size 320x180 --shot "$shot/frame.png" "$project" "$assets" 2>&1) || [ -n "$out" ]; then
    echo "$id: $out"
    failed=$((failed + 1))
  fi
done
echo "$drawn prepared, $failed failed to draw"
[ "$failed" -eq 0 ]
