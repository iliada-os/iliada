#!/usr/bin/env bash
# Sign a Nix closure and upload paths that are not already on cache.nixos.org
# or cache.iliada-os.com. Used by .github/workflows/vm.yml on push to main.
#
# Env (never printed): ILIADA_CACHE_KEY, R2_ACCOUNT_ID, R2_ACCESS_KEY_ID,
# R2_SECRET_ACCESS_KEY. Bucket: s3://iliada-cache (same as scripts/push-cache.sh).
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <store-path>..." >&2
  exit 2
fi
: "${ILIADA_CACHE_KEY:?}"
: "${R2_ACCOUNT_ID:?}"
: "${R2_ACCESS_KEY_ID:?}"
: "${R2_SECRET_ACCESS_KEY:?}"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
umask 077
# The signing key is one "name:secret" line. Write it to a 0600 file; nix
# reads the path, not the value from the command line.
printf '%s\n' "$ILIADA_CACHE_KEY" >"$work/cache.key"
chmod 600 "$work/cache.key"
export AWS_ACCESS_KEY_ID=$R2_ACCESS_KEY_ID
export AWS_SECRET_ACCESS_KEY=$R2_SECRET_ACCESS_KEY
export AWS_DEFAULT_REGION=auto
# Do not leak credentials through xtrace if a caller enabled it.
set +x

missing=$work/missing
: >"$missing"
nix path-info -r "$@" | while read -r p; do
  h=${p##*/}
  h=${h%%-*}
  nixos=$(curl -s -o /dev/null -w "%{http_code}" "https://cache.nixos.org/$h.narinfo")
  if [[ $nixos == 200 ]]; then
    continue
  fi
  iliada=$(curl -s -o /dev/null -w "%{http_code}" "https://cache.iliada-os.com/$h.narinfo")
  if [[ $iliada == 200 ]]; then
    continue
  fi
  printf '%s\n' "$p"
done >"$missing"

n=$(grep -c . "$missing" || true)
echo "$n paths to upload"
if [[ $n -eq 0 ]]; then
  echo "nothing new for cache.iliada-os.com"
  exit 0
fi

local_cache=$work/cache
# shellcheck disable=SC2046
nix copy --to "file://$local_cache?compression=zstd&secret-key=$work/cache.key" $(cat "$missing")

aws=$(nix build --no-link --print-out-paths --inputs-from . nixpkgs#awscli2 | head -1)/bin/aws
endpoint="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
up=0
while read -r p; do
  h=${p##*/}
  h=${h%%-*}
  nar=$(sed -n 's/^URL: //p' "$local_cache/$h.narinfo")
  "$aws" s3 cp --quiet "$local_cache/$nar" "s3://iliada-cache/$nar" --endpoint-url "$endpoint"
  "$aws" s3 cp --quiet "$local_cache/$h.narinfo" "s3://iliada-cache/$h.narinfo" \
    --content-type text/x-nix-narinfo --endpoint-url "$endpoint"
  up=$((up + $(stat -c %s "$local_cache/$nar")))
done <"$missing"
echo "uploaded $n paths, $((up / 1048576)) MB"
