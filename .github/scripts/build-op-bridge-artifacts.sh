#!/usr/bin/env bash
# Builds the standalone locket-op-bridge release artifacts, for users
# who install locket via cargo and fetch the bridge separately.
# Invoked by dist's extra-artifacts hook (see dist-workspace.toml).
#
# gnu and musl share one artifact per architecture: the bridge is
# libc-free (CGO_ENABLED=0), so the pairs would be byte-identical.
set -euo pipefail

version="dev"
if [[ "${GITHUB_REF_NAME:-}" == v* ]]; then
    version="${GITHUB_REF_NAME#v}"
fi

out_dir="target/op-bridge-artifacts"
mkdir -p "$out_dir"

for spec in "linux amd64 x86_64-linux" "linux arm64 aarch64-linux" \
    "darwin amd64 x86_64-macos" "darwin arm64 aarch64-macos"; do
    read -r goos goarch name <<<"$spec"
    (cd tools/op-bridge && CGO_ENABLED=0 GOOS="$goos" GOARCH="$goarch" \
        go build -trimpath -ldflags "-s -w -X main.version=$version" \
        -o "../../$out_dir/locket-op-bridge-$name" .)
    echo "built $out_dir/locket-op-bridge-$name"
done
