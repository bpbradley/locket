variable "VERSION"       { default = "0.0.0" }
variable "IS_PRERELEASE" { default = false }
variable "REGISTRY"      { default = "ghcr.io/bpbradley" }
variable "IMAGE"         { default = "locket" }
variable "PLATFORMS"     { default = "linux/amd64" }

group "release" {
  targets = ["connect", "op", "bws", "infisical", "aio"]
}

group "all" {
  targets = ["connect", "op", "bws", "infisical", "aio", "debug"]
}

target "_common" {
  context   = ".."
  dockerfile = "docker/Dockerfile"
  platforms = [PLATFORMS]
}

# Helper to generate tags conditionally based on prerelease
function "tags_for" {
  params = [suffix]
  result = concat(
    ["${REGISTRY}/${IMAGE}:${VERSION}-${suffix}"],
    IS_PRERELEASE ? [] : [
      "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}-${suffix}",
      "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}-${suffix}",
      "${REGISTRY}/${IMAGE}:${suffix}"
    ]
  )
}

function "tags_main" {
  params = []
  result = concat(
    ["${REGISTRY}/${IMAGE}:${VERSION}"],
    IS_PRERELEASE ? [] : [
      "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}",
      "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}",
      "${REGISTRY}/${IMAGE}:latest"
    ]
  )
}

target "op" {
  inherits = ["_common"]
  target = "op"
  args = {
    FEATURES = "op,exec"
    DEFAULT_PROVIDER = "op"
  }
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
  tags = tags_for("infisical")
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "aio" {
  inherits = ["_common"]
  target = "aio"
  args = {
    FEATURES = "op,connect,bws,infisical,exec"
  }
  tags = tags_main()
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "debug" {
  inherits = ["_common"]
  target = "debug"
  args = {
    FEATURES = "op,connect,bws,infisical,exec"
  }
  tags = [
    "${REGISTRY}/${IMAGE}:${VERSION}-debug",
    "${REGISTRY}/${IMAGE}:debug"
  ]
  labels = { "org.opencontainers.image.version" = VERSION }
}
