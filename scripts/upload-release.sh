#!/usr/bin/env bash
# 创建/更新 GitHub Release 并上传 dist 下的发布资产。
#
# 用法：
#   GITHUB_TOKEN=github_pat_xxx ./scripts/upload-release.sh
#
# 令牌只需要仓库的 Contents: Read and write 权限。脚本不会把令牌写入文件。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAG="${TAG:-v1.7.7}"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}"
REPO="${REPO:-Kalore-N/ASA}"
TOKEN="${GITHUB_TOKEN:?请设置 GITHUB_TOKEN（只需 Contents: Read and write）}"
API="https://api.github.com/repos/$REPO"
UPLOAD="https://uploads.github.com/repos/$REPO"
NOTES="$ROOT/packaging/release-notes-${TAG#v}.md"
if [ ! -f "$NOTES" ]; then
  NOTES="$ROOT/dist/release-notes-${TAG#v}.md"
fi
[ -f "$NOTES" ] || { echo "找不到发布说明：$NOTES" >&2; exit 1; }

umask 077
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

ASSETS=(
  "$ROOT/dist/asa-${VERSION}-linux-x86_64.tar.gz"
  "$ROOT/dist/asa-${VERSION}-windows-x86_64.zip"
  "$ROOT/dist/asa-${VERSION}-android-arm64.apk"
  "$ROOT/dist/asa-${VERSION}-android-x86_64.apk"
  "$ROOT/dist/packages/asa-${VERSION}-1-x86_64.pkg.tar.zst"
  "$ROOT/dist/packages/asa_${VERSION}_amd64.deb"
  "$ROOT/dist/packages/asa-${VERSION}-1.x86_64.rpm"
  "$ROOT/dist/SHA256SUMS-${VERSION}"
)
for asset in "${ASSETS[@]}"; do
  [ -f "$asset" ] || { echo "缺少资产：$asset" >&2; exit 1; }
done

echo "==> 查找 Release $TAG"
CODE="$(curl -sS -o "$TMP/release.json" -w '%{http_code}' \
  -H "Authorization: Bearer $TOKEN" "$API/releases/tags/$TAG")"
if [ "$CODE" = "404" ]; then
  echo "==> 创建 Release $TAG"
  python3 - "$NOTES" "$TAG" > "$TMP/create.json" <<'PY'
import json, sys
body = open(sys.argv[1], encoding="utf-8").read()
print(json.dumps({
    "tag_name": sys.argv[2],
    "target_commitish": "main",
    "name": f"ASA {sys.argv[2]}",
    "body": body,
    "draft": False,
    "prerelease": False,
}, ensure_ascii=False))
PY
  curl -sS -X POST -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" --data-binary @"$TMP/create.json" \
    "$API/releases" > "$TMP/release.json"
elif [ "$CODE" != "200" ]; then
  echo "查询 Release 失败：HTTP $CODE" >&2
  cat "$TMP/release.json" >&2
  exit 1
fi

RELEASE_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$TMP/release.json")"
echo "Release ID: $RELEASE_ID"

# 删除同名旧资产，保证脚本可重复执行。
python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
for a in d.get("assets", []):
    print(a["id"], a["name"])
' "$TMP/release.json" > "$TMP/old-assets.txt"
while read -r id name; do
  [ -n "${id:-}" ] || continue
  for asset in "${ASSETS[@]}"; do
    if [ "$(basename "$asset")" = "$name" ]; then
      echo "==> 删除旧资产 $name"
      curl -sS -X DELETE -H "Authorization: Bearer $TOKEN" \
        "$API/releases/assets/$id" >/dev/null
      break
    fi
  done
done < "$TMP/old-assets.txt"

for asset in "${ASSETS[@]}"; do
  name="$(basename "$asset")"
  echo "==> 上传 $name"
  curl -sS -X POST -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/octet-stream" \
    --data-binary @"$asset" \
    "$UPLOAD/releases/$RELEASE_ID/assets?name=$name" > "$TMP/asset.json"
  python3 - "$TMP/asset.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(f"    {d.get('name')}  {d.get('size')}  {d.get('state', d.get('message', ''))}")
PY
done

echo "==> 完成：https://github.com/$REPO/releases/tag/$TAG"
