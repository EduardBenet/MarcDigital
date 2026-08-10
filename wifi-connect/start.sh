#!/bin/sh
# Decide *when* to raise the setup portal.
#
# wifi-connect deliberately does not do this itself: run the bare binary and it
# brings up its access point immediately, tearing down the working Wi-Fi it was
# already using. Upstream's scripts/start.sh therefore makes you choose a
# connectivity condition, and this is ours.
#
# ONE deliberate difference from upstream: their script checks once at boot and
# then `sleep infinity`. That only ever provisions at startup, so a frame that
# loses its network months later - a relative changing their router, a new ISP -
# would sit offline until someone power-cycled it. This one keeps checking.
#
# The cost of looping is that a *transient* outage could strand the frame in AP
# mode, so the portal is given ACTIVITY_TIMEOUT to be used and then exits,
# letting NetworkManager retry the network it already knows.
set -eu

: "${DELAY_BEFORE_CONFIGURING:=30}"
: "${CONNECTIVITY_CHECK_INTERVAL:=30}"
: "${PORTAL_SSID:=MarcDigital Setup}"
# Seconds the portal stays up with nobody using it before giving way to a
# reconnection attempt. Long enough for someone to fetch their phone.
: "${ACTIVITY_TIMEOUT:=300}"

export PORTAL_SSID ACTIVITY_TIMEOUT
export DBUS_SYSTEM_BUS_ADDRESS="${DBUS_SYSTEM_BUS_ADDRESS:-unix:path=/host/run/dbus/system_bus_socket}"

log() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) wifi-connect: $*"; }

# Ask NetworkManager for its state rather than pinging the internet. A router
# that is up but has lost its uplink must NOT trigger provisioning: the saved
# credentials are correct, and a portal would only get in the way of a fault
# nobody at this end can fix.
is_connected() {
    case "$(nmcli -t -f STATE general 2>/dev/null || echo unknown)" in
        connected*) return 0 ;;
        *) return 1 ;;
    esac
}

log "waiting ${DELAY_BEFORE_CONFIGURING}s for a known network to associate"
sleep "$DELAY_BEFORE_CONFIGURING"

while true; do
    if is_connected; then
        sleep "$CONNECTIVITY_CHECK_INTERVAL"
        continue
    fi

    log "no connectivity - raising portal on SSID '${PORTAL_SSID}'"
    # Blocks until a network is chosen and joined, or ACTIVITY_TIMEOUT expires.
    # A failure is logged and retried rather than killing the service: this is
    # an unattended frame, and something has to keep offering the portal.
    if ./wifi-connect; then
        log "portal finished; device should now be connected"
    else
        log "portal exited non-zero (likely the activity timeout) - will re-check"
    fi
    sleep 5
done
