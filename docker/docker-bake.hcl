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
  cache-to   = ["type=gha,mode=max,scope=main"]
  cache-from = ["type=gha,scope=main"]
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

function "cache_to" {
  params = [name]
  result = ["type=gha,mode=max,scope=locket-${name}"]
}

function "cache_from" {
  params = [name]
  result = ["type=gha,scope=locket-${name}"]
}

target "op" {
  inherits = ["_common"]
  target = "op"
  args = {
    FEATURES = "op,exec"
    DEFAULT_PROVIDER = "op"
  }
  cache-to   = cache_to("op")
  cache-from = cache_from("op")
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
  cache-to   = cache_to("connect")
  cache-from = cache_from("connect")
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
  cache-to   = cache_to("bws")
  cache-from = cache_from("bws")
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
  cache-to   = cache_to("infisical")
  cache-from = cache_from("infisical")
  tags = tags_for("infisical")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "aio" {
  inherits = ["_common"]
  target = "aio"
  args = {
    FEATURES = "op,connect,bws,infisical,exec"
  }
  cache-to   = cache_to("aio")
  cache-from = cache_from("aio")
  tags = tags_main()
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "plugin" {
  inherits = ["_common"]
  target = "plugin"
  args = {
    FEATURES = "op,connect,bws,infisical,volume"
  }
  cache-to   = cache_to("volume")
  cache-from = cache_from("volume")
  tags = tags_for("volume")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "debug" {
  inherits = ["_common"]
  target = "debug"
  args = {
    FEATURES = "op,connect,bws,infisical,exec"
  }
  cache-to   = cache_to("debug")
  cache-from = cache_from("debug")
  tags = tags_for("debug")
  labels = { "org.opencontainers.image.version" = VERSION }
}
