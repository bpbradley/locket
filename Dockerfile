ARG RUST_TAG=1.91-alpine3.22
FROM rust:${RUST_TAG} AS build
WORKDIR /src
RUN apk add --no-cache musl-dev build-base pkgconfig \
 && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.lock ./
COPY . .

RUN cargo build --release --locked --target x86_64-unknown-linux-musl \
 && strip target/x86_64-unknown-linux-musl/release/secret-sidecar

FROM alpine:3.22 AS rootfs
RUN addgroup -g 65532 nonroot \
 && adduser -D -H -u 65532 -G nonroot nonroot

RUN install -d -m 1777 /tmp \
 && install -d -m 0755 /etc/ssl/certs \
 && install -d -m 0755 /usr/local/bin \
 && install -d -m 0755 /home/nonroot && chown nonroot:nonroot /home/nonroot \
 && install -d -m 0755 /templates && chown nonroot:nonroot /templates \
 && install -d -m 0755 /run/secrets && chown nonroot:nonroot /run/secrets \
 && install -d -m 0700 /op/config && chown nonroot:nonroot /op/config \
 && apk add --no-cache ca-certificates

RUN cp /etc/ssl/certs/ca-certificates.crt /ca-certificates.crt

RUN printf 'nonroot:x:65532:65532:nonroot user:/home/nonroot:/sbin/nologin\n' > /etc/passwd \
 && printf 'nonroot:x:65532:\n' > /etc/group

RUN install -d -m 1777 /tmp && stat -c '%U:%G %a %n' /tmp

FROM scratch AS base
LABEL org.opencontainers.image.title="secret-sidecar (base)"
ENV PATH=/usr/local/bin \
    HOME=/home/nonroot \
    TMPDIR=/tmp

COPY --from=rootfs /ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=rootfs /etc/passwd /etc/passwd
COPY --from=rootfs /etc/group /etc/group
COPY --from=rootfs --chown=nonroot:nonroot /home/nonroot /home/nonroot
COPY --from=rootfs --chown=nonroot:nonroot /templates /templates
COPY --from=rootfs --chown=nonroot:nonroot /run/secrets /run/secrets
COPY --from=rootfs --chmod=1777 /tmp /tmp
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/secret-sidecar /secret-sidecar

USER nonroot:nonroot
VOLUME ["/tmp", "/run/secrets", "/templates", "/op/config"]
HEALTHCHECK --interval=5s --timeout=3s --retries=30 \
  CMD ["/secret-sidecar","healthcheck"]
ENTRYPOINT ["/secret-sidecar","run"]

FROM alpine:3.22 AS opstage
ARG OP_VERSION=2.32.0
RUN set -eux; \
    apk add --no-cache ca-certificates wget; \
    echo "https://downloads.1password.com/linux/alpinelinux/stable/" >> /etc/apk/repositories; \
    wget -O /etc/apk/keys/support@1password.com-61ddfc31.rsa.pub \
      https://downloads.1password.com/linux/keys/alpinelinux/support@1password.com-61ddfc31.rsa.pub; \
    apk update; \
    apk add --no-cache 1password-cli=${OP_VERSION}-r0 || apk add --no-cache 1password-cli

FROM base AS op
LABEL org.opencontainers.image.title="secret-sidecar (op)"
COPY --from=opstage /usr/bin/op /usr/local/bin/op
COPY --from=rootfs --chown=nonroot:nonroot --chmod=700 /op/config /op/config
ENV SECRETS_PROVIDER=op \
    OP_CONFIG_DIR=/op/config

# Redundant right now as `op` is the only provider.
# But eventually the idea is that we would copy extra tools neeeded
# for all the different providers here, so that one image can be used for
# any provider.
FROM base AS aio
COPY --from=opstage /usr/bin/op /usr/local/bin/op
COPY --from=rootfs --chown=nonroot:nonroot --chmod=700 /op/config /op/config

# Debug image with full distro, extra tools, and shell access
FROM alpine:3.22 AS debug
LABEL org.opencontainers.image.title="secret-sidecar (debug)"

RUN apk add --no-cache bash curl vim tree strace jq coreutils

ENV PATH="/usr/local/bin:${PATH}" \
    HOME=/root \
    TMPDIR=/tmp \
    SECRETS_PROVIDER=op \
    OP_CONFIG_DIR=/op/config

RUN mkdir -p /run/secrets /templates /home/nonroot \
 && chmod 777 /run/secrets /templates /home/nonroot

RUN addgroup -g 65532 nonroot \
 && adduser -D -H -u 65532 -G nonroot nonroot

COPY --from=opstage /usr/bin/op /usr/local/bin/op
COPY --from=rootfs --chown=nonroot:nonroot --chmod=700 /op/config /op/config
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/secret-sidecar /usr/local/bin/secret-sidecar
VOLUME ["/tmp", "/run/secrets", "/templates", "/op/config"]
ENTRYPOINT ["/bin/bash"]
