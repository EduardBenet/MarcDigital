FROM --platform=linux/amd64 rust:1.91-bookworm AS builder

ARG RPI_TOOLS=/rpi_tools

# Let's get armv6 cross compilation working
# Enable the armhf arch
RUN dpkg --add-architecture armhf
RUN apt-get update -qq && \
    # Install the necessary packages
    # libudev-dev will also bring in the arm libc6 and gcc packages
    apt-get install -qq --no-install-recommends git pkg-config libudev-dev:armhf && \
    # Add the RPI toolchain
    git -C "/" clone -q --depth=1 https://github.com/raspberrypi/tools.git "${RPI_TOOLS}" && \
    # Remove most of the repo we just downloaded as we only need a small amount
    rm -fr "${RPI_TOOLS}/.git" \
    "${RPI_TOOLS}/arm-bcm2708/arm-bcm2708-linux-gnueabi" \
    "${RPI_TOOLS}/arm-bcm2708/arm-bcm2708hardfp-linux-gnueabi" \
    "${RPI_TOOLS}/arm-bcm2708/gcc-linaro-arm-linux-gnueabihf-raspbian" \
    "${RPI_TOOLS}/arm-bcm2708/gcc-linaro-arm-linux-gnueabihf-raspbian-x64" && \
    # Then get rid of git as we only needed it to fetch the rpi tools
    apt-get purge -qq git && \
    # Purge anything that has become useless
    apt-get autoremove -qq --purge && \
    # And finally do cleanup
    apt-get clean -qq && rm -fr /var/lib/apt/* /var/cache/apt/*

# Enable arm v6 in Rust
RUN rustup target add arm-unknown-linux-gnueabihf

# Install SDL2 dependencies
RUN apt-get update && apt-get install -y \
    libsdl2-dev:armhf \
    libsdl2-image-dev:armhf \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY .cargo ./.cargo

ENV PKG_CONFIG_ALLOW_CROSS=1
ENV PKG_CONFIG_PATH=/usr/lib/arm-linux-gnueabihf/pkgconfig

RUN cargo build --release --target=arm-unknown-linux-gnueabihf

FROM debian:bookworm-slim

# Install runtime dependencies for SDL2
RUN apt-get update && apt-get install -y \
    libsdl2-2.0-0 \
    libsdl2-image-2.0-0 \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy compiled binary from builder
COPY --from=builder /app/target/arm-unknown-linux-gnueabihf/release/MarcDigital /usr/local/bin/MarcDigital
#COPY config /config

# Set working directory
RUN mkdir -p /digital_frame/synced_photos
WORKDIR /digital_frame

# Run the MarcDigital
CMD ["/usr/local/bin/MarcDigital"]