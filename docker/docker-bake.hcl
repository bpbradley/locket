variable "VERSION"        { default = "0.0.0" }
variable "IS_PRERELEASE"  { default = false }
variable "REGISTRIES"     { default = "bpbradley" }
variable "IMAGE"          { default = "locket" }
variable "PLATFORMS"      { default = "linux/amd64" }
variable "CI"             { default = false }
variable "CACHE_REPO"     { default = "ghcr.io/bpbradley/locket" }

# Distinguishes cache refs when platforms build on separate runners.
variable "CACHE_SUFFIX"   { default = "" }

# Push images by digest only (no tags). Used by CI to build each platform on
# a native runner and stitch the manifest lists together afterwards.
variable "PUSH_BY_DIGEST" { default = false }

# Exporting cache needs package write access, which PR tokens lack.
# Reads stay enabled everywhere in CI; writes are opt-in for trusted runs
variable "CACHE_WRITE"    { default = false }

group "release" {
  targets = ["connect", "op", "bws", "infisical", "bao", "aio", "plugin"]
}

group "all" {
  targets = ["connect", "op", "bws", "infisical", "bao", "aio", "debug", "plugin"]
}

group "plugin-build" {
    targets = ["plugin"]
}

target "_common" {
  context   = ".."
  dockerfile = "docker/Dockerfile"
  platforms = split(",", PLATFORMS)
  output = PUSH_BY_DIGEST ? [digest_output()] : []
}

function "get_registries" {
  params = []
  result = split(",", REGISTRIES)
}

function "digest_output" {
  params = []
  result = "type=image,\"name=${join(",", [for reg in get_registries() : "${reg}/${IMAGE}"])}\",push-by-digest=true,name-canonical=true,push=true"
}

function "cache_ref" {
  params = [name]
  result = "${CACHE_REPO}:cache-${name}${CACHE_SUFFIX == "" ? "" : "-${CACHE_SUFFIX}"}"
}

function "cache_to_for" {
  params = [name]
  result = CACHE_WRITE ? ["type=registry,ref=${cache_ref(name)},mode=max"] : []
}

function "cache_from_for" {
  params = [name]
  result = CI ? ["type=registry,ref=${cache_ref(name)}"] : []
}

# Helper to generate tags conditionally based on prerelease
function "tags_for" {
  params = [suffix]
  result = PUSH_BY_DIGEST ? [] : flatten([
    for reg in get_registries() : concat(
      ["${reg}/${IMAGE}:${VERSION}-${suffix}"],
      IS_PRERELEASE ? [] : [
        "${reg}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}-${suffix}",
        "${reg}/${IMAGE}:${split(".", VERSION)[0]}-${suffix}",
        "${reg}/${IMAGE}:${suffix}"
      ]
    )
  ])
}

function "tags_main" {
  params = []
  result = PUSH_BY_DIGEST ? [] : flatten([
    for reg in get_registries() : concat(
      ["${reg}/${IMAGE}:${VERSION}"],
      IS_PRERELEASE ? [] : [
        "${reg}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}",
        "${reg}/${IMAGE}:${split(".", VERSION)[0]}",
        "${reg}/${IMAGE}:latest"
      ]
    )
  ])
}

target "op" {
  inherits = ["_common"]
  target = "op"
  args = {
    FEATURES = "op,exec"
    DEFAULT_PROVIDER = "op"
  }
  cache-to   = cache_to_for("op")
  cache-from = cache_from_for("op")
  tags = tags_for("op")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "connect" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "connect,exec"
    DEFAULT_PROVIDER = "op-connect"
  }
  cache-to   = cache_to_for("connect")
  cache-from = cache_from_for("connect")
  tags = tags_for("connect")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "bws" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "bws,exec"
    DEFAULT_PROVIDER = "bws"
  }
  cache-to   = cache_to_for("bws")
  cache-from = cache_from_for("bws")
  tags = tags_for("bws")
  labels = { "org.opencontainers.image.version" = VERSION }

}

target "infisical" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "infisical,exec"
    DEFAULT_PROVIDER = "infisical"
  }
  cache-to   = cache_to_for("infisical")
  cache-from = cache_from_for("infisical")
  tags = tags_for("infisical")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "bao" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "bao,exec"
    DEFAULT_PROVIDER = "bao"
  }
  cache-to   = cache_to_for("bao")
  cache-from = cache_from_for("bao")
  tags = tags_for("bao")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "aio" {
  inherits = ["_common"]
  target = "aio"
  args = {
    FEATURES = "op,connect,bws,infisical,bao,exec"
  }
  cache-to   = cache_to_for("aio")
  cache-from = cache_from_for("aio")
  tags = tags_main()
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "plugin" {
  inherits = ["_common"]
  target = "plugin"
  args = {
    FEATURES = "op,connect,bws,infisical,bao,volume"
  }
  cache-to   = cache_to_for("plugin")
  cache-from = cache_from_for("plugin")
  tags = tags_for("volume")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "debug" {
  inherits = ["_common"]
  target = "debug"
  args = {
    FEATURES = "op,connect,bws,infisical,bao,exec"
  }
  cache-to   = cache_to_for("debug")
  cache-from = cache_from_for("debug")
  tags = tags_for("debug")
  labels = { "org.opencontainers.image.version" = VERSION }
}
