# check=skip=FromPlatformFlagConstDisallowed
# ^ deliberate: see the note above the builder stage. Must stay the first line -
#   a parser directive is ignored once any comment or instruction precedes it.

# Native aarch64 build for the Raspberry Pi 4 Model B (balena device type
# `raspberrypi4-64`). Balena builds arm64 natively, so there is no cross-compile
# step: the old `raspberrypi/tools` armv6 hack and the matching
# `.cargo/config.toml` target block are gone (see REQUIREMENTS.md §8).

# Both stages pin the platform rather than inheriting it. On balena's arm64
# builders this is a no-op; anywhere else it is what stops a silent mistake.
# The runtime base is arm64-only, so with an unpinned builder an x86
# `docker compose up --build` resolves this image to amd64, compiles an x86
# binary into an arm64 image, and the failure surfaces only on the device as
# `exec format error`. Pinned, the same build emulates through QEMU (slow but
# correct) instead.
#
# `docker build --check` objects to the constant ("should not use constant
# value"), because the idiomatic form is $BUILDPLATFORM/$TARGETPLATFORM for
# images meant to be built for many architectures. This one is not: it targets
# exactly one device type, and $TARGETPLATFORM would silently fall back to the
# host's architecture whenever no --platform flag is passed - which is the exact
# bug being fixed. The warning is expected; keep the constant.
FROM --platform=linux/arm64 rust:1.97-bookworm AS builder

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
# Pinned for the same reason as the builder above - and because this image is
# published for arm64 only, so an unpinned stage on an x86 host produces an
# image whose metadata claims amd64 while its contents are arm64.
FROM --platform=linux/arm64 balenalib/raspberrypi4-64-debian:bookworm-run

# Shared libs only - SDL2 is linked dynamically (REQUIREMENTS.md §8).
#
# libgl1-mesa-dri is NOT optional despite the slideshow rendering in software:
# SDL2's KMSDRM backend creates its surface through GBM + EGL, and Mesa's EGL
# needs a DRI driver (vc4_dri.so on a Pi, swrast_dri.so as fallback). Without
# it Mesa walks vc4 -> zink -> kms_swrast -> swrast, finds nothing, and SDL
# dies with the misleading "EGL not initialized" while the display itself is
# perfectly fine. libgbm1/libegl1 come with SDL2 but are named explicitly so a
# future base-image change cannot silently drop them.
# libegl1 is only the GLVND *loader*; libegl-mesa0 is the implementation behind
# it, and SDL loads libGLESv2.so.2 (libgles2) for the KMSDRM context. Installing
# the loader without those gives "EGL not initialized" with no other clue.
RUN install_packages \
    libsdl2-2.0-0 \
    libsdl2-image-2.0-0 \
    libgl1-mesa-dri \
    libegl1 \
    libegl-mesa0 \
    libgles2 \
    libgbm1 \
    libdrm2 \
    libssl3 \
    ca-certificates

COPY --from=builder /app/target/release/MarcDigital /usr/local/bin/MarcDigital

RUN mkdir -p /digital_frame/synced_photos
WORKDIR /digital_frame

CMD ["/usr/local/bin/MarcDigital"]
