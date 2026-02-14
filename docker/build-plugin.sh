#!/bin/bash
set -euo pipefail

METADATA_FILE="bake-metadata.json"
CONFIG_SRC="${CONFIG_SRC:-./plugin/config.json}"
BUILD_DIR="./dist/plugin-build"
ROOTFS_DIR="${BUILD_DIR}/rootfs"
IS_PUSHING=false
ENABLE_FILTER=""
declare -a PLUGIN_SETTINGS=()

log() { echo -e "\033[1;34m[INFO]\033[0m $1"; }
err() { echo -e "\033[1;31m[ERROR]\033[0m $1"; exit 1; }
cleanup() { [ -n "${TEMP_CONTAINER_ID:-}" ] && docker rm -vf "$TEMP_CONTAINER_ID" >/dev/null 2>&1 || true; }
usage() {
    echo "Usage: $0 [OPTIONS]"
    echo "Options:"
    echo "  --metadata FILE   Path to metadata file (default: bake-metadata.json)"
    echo "  --config FILE     Path to config.json (default: ./plugin/config.json)"
    echo "  --push            Push the plugin after building"
    echo "  --enable STR      Enable plugin tag ending with this string"
    echo "  --set KEY=VAL     Set a plugin configuration option (can be used multiple times)"
    exit 1
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case $1 in
    --metadata)
      if [[ -z "${2:-}" ]]; then err "--metadata requires a file argument"; fi
      METADATA_FILE="$2"
      shift 2
      ;;
    --config)
      if [[ -z "${2:-}" ]]; then err "--config requires a file argument"; fi
      CONFIG_SRC="$2"
      shift 2
      ;;
    --push)
      IS_PUSHING=true
      shift
      ;;
    --enable)
      if [[ -z "${2:-}" ]]; then err "--enable requires a string argument (e.g., ':plugin')"; fi
      ENABLE_FILTER="$2"
      shift 2
      ;;
    --set)
      if [[ -z "${2:-}" ]]; then err "--set requires a KEY=VALUE argument"; fi
      PLUGIN_SETTINGS+=("$2")
      shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      err "Unknown argument: $1. Use --help for usage."
      ;;
  esac
done

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
    PLUGIN_TAG="${ARTIFACT_TAG%volume}plugin"

    SHOULD_ENABLE=false

    if [[ -n "$ENABLE_FILTER" ]]; then
        if [[ "$PLUGIN_TAG" == *"$ENABLE_FILTER" ]]; then
             SHOULD_ENABLE=true
        fi
    fi

    echo "$PLUGIN_TAG - $(date +%s)" > "$ROOTFS_DIR/.docker-plugin-build-meta"

    docker plugin rm -f "$PLUGIN_TAG" 2>/dev/null || true

    log "Creating plugin $PLUGIN_TAG..."
    docker plugin create "$PLUGIN_TAG" "$BUILD_DIR"

    if [ "$SHOULD_ENABLE" = true ]; then
        if [ ${#PLUGIN_SETTINGS[@]} -gt 0 ]; then
            log "Applying settings: ${PLUGIN_SETTINGS[*]}"
            docker plugin set "$PLUGIN_TAG" "${PLUGIN_SETTINGS[@]}"
        fi

        log "Enabling plugin $PLUGIN_TAG..."
        docker plugin enable "$PLUGIN_TAG"
    fi

    if [ "$IS_PUSHING" = true ]; then
        log "Pushing $PLUGIN_TAG..."
        docker plugin push "$PLUGIN_TAG"

        if [ "$SHOULD_ENABLE" = false ]; then
            docker plugin rm -f "$PLUGIN_TAG"
        fi
    fi
done
