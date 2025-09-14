FROM rust:1.80 AS build
WORKDIR /src
COPY Cargo.toml ./
COPY src/ src/
ENV RUSTFLAGS="-C target-cpu=native"
RUN cargo build --release

FROM 1password/op:2 AS opstage

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /
COPY --from=build   /src/target/release/secret-sidecar /secret-sidecar
COPY --from=opstage /usr/local/bin/op /usr/local/bin/op
VOLUME ["/run/secrets", "/templates"]
USER nonroot:nonroot
HEALTHCHECK --interval=5s --timeout=3s --retries=30 \
    CMD ["/secret-sidecar","healthcheck"]
ENTRYPOINT ["/secret-sidecar", "run"]
