FROM rust:1.80-alpine3.20 AS build
WORKDIR /src
RUN apk add --no-cache musl-dev build-base pkgconfig
RUN rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.lock ./
COPY . .

RUN cargo build --release --locked --target x86_64-unknown-linux-musl && \
    strip target/x86_64-unknown-linux-musl/release/secret-sidecar

FROM alpine:3.20 AS opstage
RUN set -eux; \
    apk add --no-cache ca-certificates wget; \
    echo "https://downloads.1password.com/linux/alpinelinux/stable/" >> /etc/apk/repositories; \
    wget -O /etc/apk/keys/support@1password.com-61ddfc31.rsa.pub \
      https://downloads.1password.com/linux/keys/alpinelinux/support@1password.com-61ddfc31.rsa.pub; \
    apk update; \
    apk add --no-cache 1password-cli

FROM alpine:3.20 AS rootfs
RUN adduser -D -u 65532 app \
 && install -d -m 1777 /tmp \
 && install -d -m 0755 /etc/ssl/certs \
 && install -d -m 0755 /usr/local/bin \
 && install -d -m 0755 /home/app && chown 65532:65532 /home/app \
 && install -d -m 0755 /run/secrets && chown 65532:65532 /run/secrets \
 && install -d -m 0755 /templates && chown 65532:65532 /templates

COPY --from=opstage /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

COPY --from=opstage /usr/bin/op /usr/local/bin/op
RUN chown 65532:65532 /usr/local/bin/op

FROM scratch

ENV PATH=/usr/local/bin \
    HOME=/home/app

COPY --from=rootfs / /

COPY --from=build --chown=65532:65532 \
  /src/target/x86_64-unknown-linux-musl/release/secret-sidecar /secret-sidecar

USER 65532:65532

VOLUME ["/run/secrets", "/templates"]

HEALTHCHECK --interval=5s --timeout=3s --retries=30 \
  CMD ["/secret-sidecar","healthcheck"]

ENTRYPOINT ["/secret-sidecar","run"]
