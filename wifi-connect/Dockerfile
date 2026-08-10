# Wi-Fi provisioning service, built from the upstream release.
#
# Not the balenaLabs block: its published image (bh.cr/balenalabs/wifi-connect-aarch64)
# was built 2020-12-07 and dies on current balenaOS with
#
#   Get org.freedesktop.NetworkManager.AccessPoint::RsnFlags property failed:
#   wrong property type
#
# because NetworkManager's D-Bus property types changed after that build.
#
# This mirrors upstream's own Dockerfile.template (paths, packages, layout) so it
# behaves the way the project expects. The only deliberate difference is in
# start.sh - see the comment there.

FROM balenalib/raspberrypi4-64-debian:bookworm-run

# Pin the release. Upstream's template defaults to "latest"; pinning means a
# frame in someone's house cannot change behaviour because a new version shipped.
ARG WIFI_CONNECT_VERSION=v4.11.84

# dnsmasq: wifi-connect serves DHCP and DNS while the portal is up - the captive
#   portal only works because DNS resolves everything back to the device.
# wireless-tools: iwgetid, upstream's default connectivity probe.
# network-manager: for nmcli, used by start.sh; the daemon it installs is never
#   started - the real NetworkManager runs on the host.
RUN install_packages curl ca-certificates dnsmasq wireless-tools network-manager

# Upstream's layout, and not arbitrary: wifi-connect resolves its UI directory
# *relative to the working directory*, so the binary at /usr/src/app expects the
# UI at /usr/src/app/ui. Moving either one serves a broken portal.
WORKDIR /usr/src/app

RUN set -eux; \
    base="https://github.com/balena-os/wifi-connect/releases/download/${WIFI_CONNECT_VERSION}"; \
    mkdir -p /usr/src/app/ui; \
    curl -fsSL "${base}/wifi-connect-aarch64-unknown-linux-gnu.tar.gz" \
      | tar -xz -C /usr/src/app; \
    curl -fsSL "${base}/wifi-connect-ui.tar.gz" \
      | tar -xz -C /usr/src/app/ui; \
    chmod +x /usr/src/app/wifi-connect; \
    test -x /usr/src/app/wifi-connect; \
    test -f /usr/src/app/ui/index.html

COPY start.sh /usr/src/app/start.sh
RUN chmod +x /usr/src/app/start.sh

CMD ["/usr/src/app/start.sh"]
