variable "VERSION"   { default = "0.0.0"}
variable "REGISTRY"  { default = "ghcr.io/bpbradley" }
variable "IMAGE"     { default = "secret-sidecar" }
variable "PLATFORMS" { default = "linux/amd64" }

group "release" {
  targets = ["base", "op", "aio"]
}

target "_common" {
  context   = "."
  platforms = [PLATFORMS]
}

target "base" {
  inherits = ["_common"]
  target   = "base"
  tags = [
    "${REGISTRY}/${IMAGE}:${VERSION}-base",
    "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}-base",
    "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}-base",
    "${REGISTRY}/${IMAGE}:base",
  ]
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "op" {
  inherits = ["_common"]
  target   = "op"
  tags = [
    "${REGISTRY}/${IMAGE}:${VERSION}-op",
    "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}-op",
    "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}-op",
    "${REGISTRY}/${IMAGE}:op"
  ]
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "aio" {
  inherits = ["_common"]
  target   = "aio"
  tags = [
    "${REGISTRY}/${IMAGE}:${VERSION}",
    "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}.${split(".", VERSION)[1]}",
    "${REGISTRY}/${IMAGE}:${split(".", VERSION)[0]}",
    "${REGISTRY}/${IMAGE}:latest",
  ]
  labels = { "org.opencontainers.image.version" = VERSION }
}

target "debug" {
  inherits = ["_common"]
  target   = "debug"
  tags = [
    "${REGISTRY}/${IMAGE}:${VERSION}-debug",
    "${REGISTRY}/${IMAGE}:debug",
    "${REGISTRY}/${IMAGE}:latest-debug"
  ]
  labels = { "org.opencontainers.image.version" = VERSION }
}
