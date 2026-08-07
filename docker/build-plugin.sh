#!/bin/bash
set -euo pipefail

METADATA_FILE="bake-metadata.json"
CONFIG_SRC="${CONFIG_SRC:-./plugin/config.json}"
BUILD_DIR="./dist/plugin-build"
ROOTFS_DIR="${BUILD_DIR}/rootfs"
IS_PUSHING=false
ENABLE_FILTER=""
PLATFORM=""
TAG_SUFFIX=""
ALIAS_UNQUALIFIED=false
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
    echo "  --platform VALUE  Source image platform, e.g. linux/arm64"
    echo "  --tag-suffix STR  Append a suffix to generated plugin tags"
    echo "  --alias-unqualified  Also create the unsuffixed plugin tags"
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
    --platform)
      if [[ -z "${2:-}" ]]; then err "--platform requires a value (e.g., linux/arm64)"; fi
      PLATFORM="$2"
      shift 2
      ;;
    --tag-suffix)
      if [[ -z "${2:-}" ]]; then err "--tag-suffix requires a value (e.g., -arm64)"; fi
      TAG_SUFFIX="$2"
      shift 2
      ;;
    --alias-unqualified)
      ALIAS_UNQUALIFIED=true
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

if [[ "$ALIAS_UNQUALIFIED" == true && -z "$TAG_SUFFIX" ]]; then
    err "--alias-unqualified requires --tag-suffix"
fi

command -v jq >/dev/null 2>&1 || err "jq is required."
[ -f "$CONFIG_SRC" ] || err "Config file not found: $CONFIG_SRC"
[ -f "$METADATA_FILE" ] || err "Metadata file not found: $METADATA_FILE"

log "Reading build metadata from $METADATA_FILE..."
ARTIFACT_TAGS=()
while IFS= read -r line; do
    [ -n "$line" ] && ARTIFACT_TAGS+=("$line")
done < <(jq -r '.plugin."image.name" | split(",")[]' "$METADATA_FILE")

if [ ${#ARTIFACT_TAGS[@]} -eq 0 ]; then
    err "No tags found in metadata for target 'plugin'."
fi

SRC_IMAGE="${ARTIFACT_TAGS[0]}"
log "Source Artifact: $SRC_IMAGE"

rm -rf "$BUILD_DIR"
mkdir -p "$ROOTFS_DIR"

if [[ -n "$PLATFORM" ]]; then
    TARGET_PLATFORM="$PLATFORM"
else
    TARGET_PLATFORM=$(docker info --format '{{.OSType}}/{{.Architecture}}')
    case "$TARGET_PLATFORM" in
        */x86_64) TARGET_PLATFORM="${TARGET_PLATFORM%/x86_64}/amd64" ;;
        */aarch64|*/arm64|*/armv8l) TARGET_PLATFORM="${TARGET_PLATFORM%/*}/arm64" ;;
        */armv7l) TARGET_PLATFORM="${TARGET_PLATFORM%/armv7l}/arm/v7" ;;
        */armv6l) TARGET_PLATFORM="${TARGET_PLATFORM%/armv6l}/arm/v6" ;;
        */amd64|*/arm/v7|*/arm/v6|*/386|*/riscv64|*/ppc64le|*/s390x) ;;
        *) err "Unsupported Docker daemon architecture: '$TARGET_PLATFORM'" ;;
    esac
fi

image_platform=$(docker image inspect "$SRC_IMAGE" \
    --format '{{.Os}}/{{.Architecture}}' 2>/dev/null || true)

if [[ "$image_platform" == "$TARGET_PLATFORM" ]]; then
    log "Image found locally for $TARGET_PLATFORM."
else
    log "Pulling $SRC_IMAGE for $TARGET_PLATFORM..."
    docker pull --platform "$TARGET_PLATFORM" "$SRC_IMAGE"
fi

TEMP_CONTAINER_ID=$(docker create --platform "$TARGET_PLATFORM" "$SRC_IMAGE" true)
docker export "$TEMP_CONTAINER_ID" | tar -x -C "$ROOTFS_DIR"
cp "$CONFIG_SRC" "$BUILD_DIR/"

for ARTIFACT_TAG in "${ARTIFACT_TAGS[@]}"; do
    case "$ARTIFACT_TAG" in
        *-volume) PLUGIN_TAG_BASE="${ARTIFACT_TAG%-volume}-plugin" ;;
        *:volume) PLUGIN_TAG_BASE="${ARTIFACT_TAG%:volume}:plugin" ;;
        *) err "Tag transformation failed: '$ARTIFACT_TAG' must end with -volume or :volume" ;;
    esac

    PLUGIN_TAGS=("${PLUGIN_TAG_BASE}${TAG_SUFFIX}")
    if [[ "$ALIAS_UNQUALIFIED" == true ]]; then
        PLUGIN_TAGS+=("$PLUGIN_TAG_BASE")
    fi

    for PLUGIN_TAG in "${PLUGIN_TAGS[@]}"; do
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
done
