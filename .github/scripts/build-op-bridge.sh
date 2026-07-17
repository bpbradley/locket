#!/usr/bin/env bash
# Builds locket-op-bridge for each given Rust target triple and exports
# LOCKET_OP_BRIDGE_BIN_<TRIPLE> via GITHUB_ENV so that locket's build.rs
# embeds the matching bridge. Invoked from .github/build-setup.yml inside
# the dist release workflow.
set -euo pipefail

version="dev"
if [[ "${GITHUB_REF_NAME:-}" == v* ]]; then
    version="${GITHUB_REF_NAME#v}"
fi

out_root="${RUNNER_TEMP:-/tmp}/op-bridge"

for triple in "$@"; do
    case "$triple" in
        x86_64-*) goarch=amd64 ;;
        aarch64-*) goarch=arm64 ;;
        *)
            echo "no op bridge mapping for $triple, skipping"
            continue
            ;;
    esac
    case "$triple" in
        *-apple-darwin) goos=darwin ;;
        *-linux-*) goos=linux ;;
        *)
            echo "no op bridge mapping for $triple, skipping"
            continue
            ;;
    esac

    out="$out_root/$triple/locket-op-bridge"
    mkdir -p "$(dirname "$out")"
    (cd tools/op-bridge && CGO_ENABLED=0 GOOS="$goos" GOARCH="$goarch" \
        go build -trimpath -ldflags "-s -w -X main.version=$version" -o "$out" .)

    key="LOCKET_OP_BRIDGE_BIN_$(echo "$triple" | tr '[:lower:]' '[:upper:]' | tr -- '-.' '__')"
    echo "$key=$out" >>"$GITHUB_ENV"
    echo "built $out ($goos/$goarch, version $version)"
done
