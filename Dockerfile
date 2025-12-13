FROM rust:1.91-bookworm AS builder

# Install SDL2 dependencies
RUN apt-get update && apt-get install -y \
    libsdl2-dev \
    libsdl2-image-dev \
    pkg-config \
    cmake \
    git \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
#COPY config ./config

RUN cargo build --release

# FROM balenalib/raspberrypi4-64-debian:latest
FROM debian:bookworm-slim

# Install runtime dependencies for SDL2
RUN apt-get update && apt-get install -y \
    libsdl2-2.0-0 \
    libsdl2-image-2.0-0 \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy compiled binary from builder
COPY --from=builder /app/target/release/MarcDigital /usr/local/bin/MarcDigital
#COPY config /config

# Set working directory
RUN mkdir -p /digital_frame/synced_photos
WORKDIR /digital_frame

# Run the MarcDigital
CMD ["/usr/local/bin/MarcDigital"]