# syntax=docker/dockerfile:1
FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets/litecom-logo.png ./assets/litecom-logo.png
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --locked

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir /data && chown 65532:65532 /data
COPY --from=build /build/target/release/litecom /usr/local/bin/litecom
COPY LICENSE /usr/share/doc/litecom/LICENSE
COPY THIRD-PARTY-NOTICES.html /usr/share/doc/litecom/THIRD-PARTY-NOTICES.html
USER 65532:65532
ENV LITECOM_HOME=/data/config
WORKDIR /data
EXPOSE 4141
ENTRYPOINT ["/usr/local/bin/litecom"]
CMD ["--help"]
