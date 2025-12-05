ARG RUST_TAG=1.91-alpine3.22
FROM rust:${RUST_TAG} AS build
WORKDIR /src
RUN apk add --no-cache musl-dev build-base pkgconfig \
 && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.lock ./
COPY . .

ARG FEATURES="op,connect,bws"

RUN cargo build --release --locked --target x86_64-unknown-linux-musl \
  --no-default-features --features "${FEATURES}" \
 && strip target/x86_64-unknown-linux-musl/release/locket

FROM alpine:3.22 AS rootfs
RUN addgroup -g 65532 nonroot \
 && adduser -D -H -u 65532 -G nonroot nonroot

RUN grep -E '^(root|nonroot)' /etc/passwd > /etc/passwd.min \
 && grep -E '^(root|nonroot)' /etc/group > /etc/group.min

RUN install -d -m 1777 /tmp \
 && install -d -m 0755 /etc/ssl/certs \
 && install -d -m 0755 /usr/local/bin \
 && install -d -m 0755 /home/nonroot && chown nonroot:nonroot /home/nonroot \
 && install -d -m 0755 /templates && chown nonroot:nonroot /templates \
 && install -d -m 0755 /run/secrets && chown nonroot:nonroot /run/secrets \
 && install -d -m 0700 /config/op && chown nonroot:nonroot /config/op \
 && apk add --no-cache ca-certificates

RUN cp /etc/ssl/certs/ca-certificates.crt /ca-certificates.crt

FROM scratch AS base
LABEL org.opencontainers.image.title="locket (base)"

ARG DEFAULT_PROVIDER
ENV SECRETS_PROVIDER=${DEFAULT_PROVIDER}

ENV PATH=/usr/local/bin \
    HOME=/home/nonroot \
    TMPDIR=/tmp \
    XDG_CONFIG_HOME=/config \
    HOME=/home/nonroot

COPY --from=rootfs /ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=rootfs /etc/passwd /etc/passwd
COPY --from=rootfs /etc/group /etc/group
COPY --from=rootfs --chown=nonroot:nonroot /home/nonroot /home/nonroot
COPY --from=rootfs --chown=nonroot:nonroot /templates /templates
COPY --from=rootfs --chown=nonroot:nonroot /run/secrets /run/secrets
COPY --from=rootfs --chmod=1777 /tmp /tmp
COPY --from=rootfs --chmod=644 /etc/passwd.min /etc/passwd
COPY --from=rootfs --chmod=644 /etc/group.min /etc/group
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/locket /usr/local/bin/locket

WORKDIR /
USER nonroot:nonroot
VOLUME ["/run/secrets/locket", "/templates"]
HEALTHCHECK --interval=5s --timeout=3s --retries=30 \
  CMD ["locket","healthcheck"]
ENTRYPOINT ["locket","run"]

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
LABEL org.opencontainers.image.title="locket (op)"
COPY --from=opstage /usr/bin/op /usr/local/bin/op
COPY --from=rootfs --chown=nonroot:nonroot --chmod=700 /config/op /config/op

# Redundant right now as `op` is the only provider which requires extra tools.
# But eventually the idea is that we would copy extra tools neeeded
# for all the different providers here, so that one image can be used for
# any provider.
FROM base AS aio
COPY --from=opstage /usr/bin/op /usr/local/bin/op
COPY --from=rootfs --chown=nonroot:nonroot --chmod=700 /config/op /config/op

# Debug image with full distro, extra tools, and shell access
FROM alpine:3.22 AS debug
LABEL org.opencontainers.image.title="locket (debug)"

RUN apk add --no-cache bash curl vim tree strace jq coreutils

ENV PATH="/usr/local/bin:${PATH}" \
    HOME=/root \
    TMPDIR=/tmp \
    XDG_CONFIG_HOME=/config

RUN mkdir -p /run/secrets /templates /config/op \
 && chmod 740 /run/secrets /templates /config/op

RUN addgroup -g 65532 nonroot \
 && adduser -D -H -u 65532 -G nonroot nonroot

COPY --from=opstage /usr/bin/op /usr/local/bin/op
COPY --from=rootfs --chown=nonroot:nonroot --chmod=700 /config/op /config/op
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/locket /usr/local/bin/locket
VOLUME ["/run/secrets/locket", "/templates"]
ENTRYPOINT ["/bin/bash"]
