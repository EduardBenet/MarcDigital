# MarcDigital

[![CI](https://github.com/EduardBenet/MarcDigital/actions/workflows/main.yml/badge.svg?branch=rust)](https://github.com/EduardBenet/MarcDigital/actions/workflows/main.yml)
[![Release](https://github.com/EduardBenet/MarcDigital/actions/workflows/release.yml/badge.svg)](https://github.com/EduardBenet/MarcDigital/actions/workflows/release.yml)

A digital photo frame for older family members: relatives push photos to the cloud, the
frame syncs them locally and shows a rotating fullscreen slideshow. Runs on a Raspberry Pi,
fully automated. Written entirely in **Rust**.

> **See [`REQUIREMENTS.md`](./REQUIREMENTS.md) for the authoritative project spec** —
> requirements, target hardware, status, and known gaps.

## Layout
Two crates. `core` holds the pure logic and carries the test suite; it depends on
neither SDL2 nor the Azure SDK, so `cargo test -p marcdigital-core` runs anywhere.

- `core/src/config.rs` — typed config from env, fails fast on anything missing
- `core/src/sync.rs` — the cost-critical diff (`plan_sync`) + sync orchestration
- `core/src/store.rs` — the `PhotoStore` trait the sync logic is written against
- `src/store.rs` — Azure Blob implementation of `PhotoStore` (Entra service principal)
- `src/main.rs` — SDL2 fullscreen slideshow; wires config → sync → slideshow

## Configuration
Everything comes from environment variables — no defaults for secrets, and the app
refuses to start if any are missing. Local dev uses a gitignored `.env` (copy
`.env.example`); on balenaCloud these are fleet/device service variables.

| Variable | Meaning |
|---|---|
| `AZURE_TENANT_ID` | Entra tenant of the service principal |
| `AZURE_CLIENT_ID` | Service-principal app ID |
| `AZURE_CLIENT_SECRET` | Service-principal secret (do **not** commit) |
| `AZURE_STORAGE_ACCOUNT` | Azure storage account name |
| `CONTAINER_NAME` | Blob container holding the photos |

Optional, with defaults:

| Variable | Default | Meaning |
|---|---|---|
| `SYNCED_PHOTOS_DIR` | `./synced_photos` | Where synced photos are kept |
| `ROTATION_SECONDS` | `30` | Seconds each photo is shown |
| `SYNC_INTERVAL_SECONDS` | `1800` | How often the frame re-syncs from Azure |
| `MARCDIGITAL_VERBOSE` | unset | `1`/`true`/`yes`/`on` enables verbose diagnostics |
| `SDL_VIDEODRIVER` | `kmsdrm` (set in compose) | Overrides the SDL video backend |
| `SDL_RENDER_DRIVER` | unset | Overrides the renderer; unset means the app picks `opengles2` |

**`MARCDIGITAL_VERBOSE`** is the one to reach for when the panel misbehaves. By
default the frame logs only what matters — sync results, the photo count at boot,
the renderer in use, and anything that went wrong — because a 30-second rotation
would otherwise write ~2,900 lines a day for the life of the device. Setting it
adds the boot-time hardware dump (`/dev/dri` contents, console devices, SDL video
and render driver lists, display enumeration) and a line per photo shown. Warnings
and errors are never suppressed.

Photos are synced into `./synced_photos`. There is no manifest file — the folder
contents *are* the manifest, so there is no separate state to drift out of sync.
Downloads are written to `name.tmp` and renamed into place, so an interrupted
download can never leave a truncated JPEG under a real filename.

## Build & run
Local build:
```
cargo build --release
```

The container targets **aarch64**. `docker-compose.yaml` is what balena builds from
(`git push balena <branch>:master`); balena's builders are arm64, so there is no
cross-compile step there.

Do **not** run `docker compose up --build` on an x86 dev machine: the builder stage
has no `--platform`, so it would produce an x86 binary inside an arm64 runtime image
and fail with `exec format error`. To build the real image locally, emulate:
```
docker buildx build --platform linux/arm64 -t marcdigital:arm64 .
```
Expect this to be slow — the Azure SDK's crypto stack compiles C under QEMU.

## Testing
```
cargo test --workspace
```
The `core` tests need no network and no system libraries. Building `src/` (the SDL2
binary) requires `libsdl2-dev` + `libsdl2-image-dev`.

## Status
Running on hardware: a Pi 4 in the balena fleet boots, authenticates to Azure with the
service principal, syncs, and rotates photos fullscreen on a DSI panel. See
`IMPLEMENTATION_PLAN.md` for the phased plan and `REQUIREMENTS.md` for the spec.

Working: periodic background sync (the display opens first, so a frame that boots
before Wi-Fi shows its existing photos immediately); atomic downloads; per-blob error
recovery; one texture held at a time, downscaled to the panel; corrupt files skipped
and remembered; a waiting screen when there are no photos; and retry-instead-of-exit
when the display is not ready.

Remaining gaps:

- No integration tests against Azurite, and `src/store.rs` (the Azure client) has no
  tests at all (Phase 3.2/3.3).
- CI runs fmt/clippy/test/build but no arm64 image build and no secret scan (Phase 7).
- The leaked SAS is still in git history; the token is expired, but the purge
  (Phase 0.3) has not been run.
- Only one device is deployed; multi-device and reboot-survival checks are untested
  (Phase 8.5/8.6).
- Out of scope by design: first-boot Wi-Fi onboarding (balena handles it) and
  prev/next navigation buttons (advance is purely time-based).

### Display notes (hard-won)
The DSI panel needs all of: the `vc4-kms-dsi-*` overlay via
`BALENA_HOST_CONFIG_dtoverlay` (Configuration tab, **not** Variables); Mesa in the
runtime image (`libgl1-mesa-dri`, `libegl-mesa0`, `libgles2`); the `opengles2`
renderer explicitly, since SDL otherwise picks desktop `opengl`, which reports
`accelerated: true` and never reaches the screen; and a `present()` every frame,
because KMSDRM page-flips and a single draw stays in a back buffer.

## License
MIT — see `LICENSE.md`.
