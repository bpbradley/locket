#!/bin/bash
set -euo pipefail

REGISTRY="${REGISTRY:-ghcr.io/bpbradley}"
PLUGIN_NAME="${PLUGIN_NAME:-locket}"
VERSION="${VERSION:-$(cargo run --quiet -- --version | cut -d' ' -f2)}"

PLUGIN_TAG="${REGISTRY}/${PLUGIN_NAME}:plugin"
BAKED_IMAGE_TAG="${REGISTRY}/${PLUGIN_NAME}:${VERSION}-plugin"

BUILD_DIR="./dist/plugin-build"
ROOTFS_DIR="${BUILD_DIR}/rootfs"
CONFIG_SRC="./plugin/config.json"

log() { echo -e "\033[1;34m[INFO]\033[0m $1"; }
err() { echo -e "\033[1;31m[ERROR]\033[0m $1"; exit 1; }

cleanup() {
    if [ -n "${TEMP_CONTAINER_ID:-}" ]; then
        docker rm -vf "$TEMP_CONTAINER_ID" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

[ -f "$CONFIG_SRC" ] || err "Config file not found at $CONFIG_SRC"

log "Starting Plugin Build for ${PLUGIN_TAG} (Version: ${VERSION})"

log "Baking plugin image..."
export VERSION
docker buildx bake --allow=fs.read=.. -f ./docker-bake.hcl plugin --load

log "Extracting RootFS..."
rm -rf "$BUILD_DIR"
mkdir -p "$ROOTFS_DIR"

TEMP_CONTAINER_ID=$(docker create "$BAKED_IMAGE_TAG" true)

docker export "$TEMP_CONTAINER_ID" | tar -x -C "$ROOTFS_DIR"

log "Applying configuration..."
cp "$CONFIG_SRC" "$BUILD_DIR/"

log "Creating Docker Plugin..."

docker plugin rm -f "$PLUGIN_TAG" 2>/dev/null || true
docker plugin create "$PLUGIN_TAG" "$BUILD_DIR"

log "Plugin created successfully: ${PLUGIN_TAG}"

if [[ "${1:-}" == "--push" ]]; then
    docker plugin push "$PLUGIN_TAG"
fi
