#!/bin/bash
set -euo pipefail

METADATA_FILE="${1:-bake-metadata.json}"
CONFIG_SRC="${CONFIG_SRC:-./plugin/config.json}"
BUILD_DIR="./dist/plugin-build"
ROOTFS_DIR="${BUILD_DIR}/rootfs"
IS_PUSHING=false

if [[ "${2:-}" == "--push" ]]; then IS_PUSHING=true; fi

log() { echo -e "\033[1;34m[INFO]\033[0m $1"; }
err() { echo -e "\033[1;31m[ERROR]\033[0m $1"; exit 1; }
cleanup() { [ -n "${TEMP_CONTAINER_ID:-}" ] && docker rm -vf "$TEMP_CONTAINER_ID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

command -v jq >/dev/null 2>&1 || err "jq is required."
[ -f "$CONFIG_SRC" ] || err "Config file not found: $CONFIG_SRC"
[ -f "$METADATA_FILE" ] || err "Metadata file not found: $METADATA_FILE"

log "Reading build metadata from $METADATA_FILE..."
mapfile -t ARTIFACT_TAGS < <(jq -r '.plugin."image.name" | split(",")[]' "$METADATA_FILE")

if [ ${#ARTIFACT_TAGS[@]} -eq 0 ]; then
    err "No tags found in metadata for target 'plugin'."
fi

SRC_IMAGE="${ARTIFACT_TAGS[0]}"
log "Source Artifact: $SRC_IMAGE"

rm -rf "$BUILD_DIR"
mkdir -p "$ROOTFS_DIR"

if docker image inspect "$SRC_IMAGE" >/dev/null 2>&1; then
    log "Image found locally."
else
    log "Pulling $SRC_IMAGE..."
    docker pull "$SRC_IMAGE"
fi

TEMP_CONTAINER_ID=$(docker create "$SRC_IMAGE" true)
docker export "$TEMP_CONTAINER_ID" | tar -x -C "$ROOTFS_DIR"
cp "$CONFIG_SRC" "$BUILD_DIR/"

for ARTIFACT_TAG in "${ARTIFACT_TAGS[@]}"; do
    if [[ "$ARTIFACT_TAG" == *":volume" ]]; then
        PLUGIN_TAG="${ARTIFACT_TAG%:volume}:plugin"
    else
        PLUGIN_TAG="${ARTIFACT_TAG%-volume}"
    fi

    echo "$PLUGIN_TAG - $(date +%s)" > "$ROOTFS_DIR/.docker-plugin-build-meta"
    docker plugin rm -f "$PLUGIN_TAG" 2>/dev/null || true
    docker plugin create "$PLUGIN_TAG" "$BUILD_DIR"

    if [ "$IS_PUSHING" = true ]; then
        log "Pushing $PLUGIN_TAG..."
        docker plugin push "$PLUGIN_TAG"
        docker plugin rm -f "$PLUGIN_TAG"
    fi
done
