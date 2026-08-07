# Native aarch64 build for the Raspberry Pi 4 Model B (balena device type
# `raspberrypi4-64`). Balena builds arm64 natively, so there is no cross-compile
# step: the old `raspberrypi/tools` armv6 hack and the matching
# `.cargo/config.toml` target block are gone (see REQUIREMENTS.md §8).

FROM rust:1.97-bookworm AS builder

# SDL2 headers to compile against, plus cmake/pkg-config for the native deps
# (aws-lc-sys, pulled in by the Azure SDK's rustls stack, needs cmake).
RUN apt-get update -qq && \
    apt-get install -y -qq --no-install-recommends \
        libsdl2-dev \
        libsdl2-image-dev \
        cmake \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# The workspace has two members (Cargo.toml `members = ["core"]`), so `core`
# must be copied or cargo cannot resolve the workspace at all.
COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY src ./src

RUN cargo build --release --locked

# Runtime: balena's Debian bookworm base, same glibc as the builder image.
FROM balenalib/raspberrypi4-64-debian:bookworm-run

# Shared libs only — SDL2 is linked dynamically (REQUIREMENTS.md §8).
RUN install_packages \
    libsdl2-2.0-0 \
    libsdl2-image-2.0-0 \
    libssl3 \
    ca-certificates

COPY --from=builder /app/target/release/MarcDigital /usr/local/bin/MarcDigital

RUN mkdir -p /digital_frame/synced_photos
WORKDIR /digital_frame

CMD ["/usr/local/bin/MarcDigital"]
