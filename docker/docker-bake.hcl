variable "VERSION"       { default = "0.0.0" }
variable "IS_PRERELEASE" { default = false }
variable "REGISTRIES"    { default = "bpbradley" }
variable "IMAGE"         { default = "locket" }
variable "PLATFORMS"     { default = "linux/amd64" }

group "release" {
  targets = ["connect", "op", "bws", "infisical", "aio", "plugin"]
}

group "all" {
  targets = ["connect", "op", "bws", "infisical", "aio", "debug", "plugin"]
}

group "plugin-build" {
    targets = ["plugin"]
}

target "_common" {
  context   = ".."
  dockerfile = "docker/Dockerfile"
  platforms = [PLATFORMS]
}

function "get_registries" {
  params = []
  result = split(",", REGISTRIES)
}

# Helper to generate tags conditionally based on prerelease
function "tags_for" {
  params = [suffix]
  result = flatten([
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
  result = flatten([
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

function "gha_cache" {
  params = [scope_name]
  result = {
    cache-to   = ["type=gha,mode=max,scope=${scope_name}"]
    cache-from = ["type=gha,scope=${scope_name}"]
  }
}

target "op" {
  inherits = ["_common"]
  target = "op"
  args = {
    FEATURES = "op,exec"
    DEFAULT_PROVIDER = "op"
  }
  tags = tags_for("op")
  cache-from = gha_cache("op").cache-from
  cache-to   = gha_cache("op").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "connect" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "connect,exec"
    DEFAULT_PROVIDER = "op-connect"
  }
  tags = tags_for("connect")
  cache-from = gha_cache("connect").cache-from
  cache-to   = gha_cache("connect").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "bws" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "bws,exec"
    DEFAULT_PROVIDER = "bws"
  }
  tags = tags_for("bws")
  cache-from = gha_cache("bws").cache-from
  cache-to   = gha_cache("bws").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }

}

target "infisical" {
  inherits = ["_common"]
  target = "base"
  args = {
    FEATURES = "infisical,exec"
    DEFAULT_PROVIDER = "infisical"
  }
  tags = tags_for("infisical")
  cache-from = gha_cache("infisical").cache-from
  cache-to   = gha_cache("infisical").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "aio" {
  inherits = ["_common"]
  target = "aio"
  args = {
    FEATURES = "op,connect,bws,infisical,exec"
  }
  tags = tags_main()
  cache-from = gha_cache("aio").cache-from
  cache-to   = gha_cache("aio").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "plugin" {
  inherits = ["_common"]
  target = "plugin"
  args = {
    FEATURES = "op,connect,bws,infisical,volume"
  }
  tags = tags_for("volume")
  cache-from = gha_cache("volume").cache-from
  cache-to   = gha_cache("volume").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "debug" {
  inherits = ["_common"]
  target = "debug"
  args = {
    FEATURES = "op,connect,bws,infisical,exec"
  }
  tags = tags_for("debug")
  cache-from = gha_cache("debug").cache-from
  cache-to   = gha_cache("debug").cache-to
  labels = { "org.opencontainers.image.version" = VERSION }
}
