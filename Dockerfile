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

# Shared libs only - SDL2 is linked dynamically (REQUIREMENTS.md §8).
#
# libgl1-mesa-dri is NOT optional despite the slideshow rendering in software:
# SDL2's KMSDRM backend creates its surface through GBM + EGL, and Mesa's EGL
# needs a DRI driver (vc4_dri.so on a Pi, swrast_dri.so as fallback). Without
# it Mesa walks vc4 -> zink -> kms_swrast -> swrast, finds nothing, and SDL
# dies with the misleading "EGL not initialized" while the display itself is
# perfectly fine. libgbm1/libegl1 come with SDL2 but are named explicitly so a
# future base-image change cannot silently drop them.
RUN install_packages \
    libsdl2-2.0-0 \
    libsdl2-image-2.0-0 \
    libgl1-mesa-dri \
    libegl1 \
    libgbm1 \
    libssl3 \
    ca-certificates

COPY --from=builder /app/target/release/MarcDigital /usr/local/bin/MarcDigital

RUN mkdir -p /digital_frame/synced_photos
WORKDIR /digital_frame

CMD ["/usr/local/bin/MarcDigital"]
