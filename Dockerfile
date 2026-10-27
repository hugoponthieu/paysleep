# syntax=docker/dockerfile:1.7

FROM debian:trixie-slim AS proton-drive-downloader

ARG PROTON_DRIVE_VERSION=0.9.0

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN curl --fail --location \
    --proto '=https' \
    --retry 5 \
    --retry-all-errors \
    --show-error \
    --silent \
    "https://proton.me/download/drive/cli/${PROTON_DRIVE_VERSION}/linux-x64/proton-drive" \
    --output /usr/local/bin/proton-drive \
    && chmod 0755 /usr/local/bin/proton-drive

FROM rust:1.90-trixie AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY paysleep-app ./paysleep-app
COPY paysleep-application ./paysleep-application
COPY paysleep-core ./paysleep-core
COPY paysleep-storage-filesystem ./paysleep-storage-filesystem
COPY paysleep-storage-google-drive ./paysleep-storage-google-drive
COPY paysleep-storage-proton-drive ./paysleep-storage-proton-drive
COPY paysleep-web ./paysleep-web
COPY proton-drive ./proton-drive

RUN --mount=type=cache,id=paysleep-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
  --mount=type=cache,id=paysleep-cargo-git,target=/usr/local/cargo/git/db,sharing=locked \
  --mount=type=cache,id=paysleep-target,target=/build/target,sharing=locked \
  cargo build --locked --release -p paysleep-app \
  && install -Dm755 target/release/paysleep-app /out/paysleep-app \
  && install -d -m755 /out/data

FROM gcr.io/distroless/cc-debian13:nonroot

COPY --from=builder --chown=10001:10001 /out/paysleep-app /usr/local/bin/paysleep-app
COPY --from=proton-drive-downloader /usr/local/bin/proton-drive /usr/local/bin/proton-drive
COPY --from=builder --chown=10001:10001 /out/data/ /var/lib/paysleep/

USER 10001:10001
WORKDIR /var/lib/paysleep

ENV PAYSLEEP_DATA_DIR=/var/lib/paysleep \
    PAYSLEEP_BIND=0.0.0.0:8080 \
    PAYSLEEP_PROTON_BIN=/usr/local/bin/proton-drive \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

EXPOSE 8080
VOLUME ["/var/lib/paysleep"]

ENTRYPOINT ["/usr/local/bin/paysleep-app"]
