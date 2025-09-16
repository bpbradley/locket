FROM rust:1.89-alpine3.20 AS build
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
VOLUME ["/tmp", "/run/secrets", "/templates"]
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
ENV SECRETS_PROVIDER=op

# Redundant right now as `op` is the only provider.
# But eventually the idea is that we would copy extra tools neeeded
# for all the different providers here, so that one image can be used for
# any provider.
FROM base AS aio
COPY --from=opstage /usr/bin/op /usr/local/bin/op