#!/usr/bin/env sh
set -eu

binary_path="${1:?usage: check-binary-size.sh <binary> [limit-bytes]}"
limit_bytes="${2:-52428800}"
actual_bytes="$(wc -c < "$binary_path" | tr -d ' ')"

echo "$binary_path: $actual_bytes bytes (limit: $limit_bytes bytes)"
if [ "$actual_bytes" -gt "$limit_bytes" ]; then
  echo "Release binary exceeds the configured size limit." >&2
  exit 1
fi
