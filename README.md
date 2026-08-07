# MarcDigital

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

Optional, with defaults: `SYNCED_PHOTOS_DIR` (`./synced_photos`),
`ROTATION_SECONDS` (`30`), `SYNC_INTERVAL_SECONDS` (`1800`).

Photos are synced into `./synced_photos`. There is no manifest file — the folder
contents *are* the manifest, so there is no separate state to drift out of sync.

## Build & run
Local build:
```
cargo build --release
```

Container build for the Pi 4 (aarch64) — balena builds this natively, so there is
no cross-compile step:
```
docker compose up --build
```
To build the same image on an x86 dev machine (via QEMU emulation):
```
docker buildx build --platform linux/arm64 -t marcdigital:arm64 .
```

## Testing
```
cargo test --workspace
```
The `core` tests need no network and no system libraries. Building `src/` (the SDL2
binary) requires `libsdl2-dev` + `libsdl2-image-dev`.

## Status
Slideshow and Azure sync work. See `IMPLEMENTATION_PLAN.md` for the phased plan and
`REQUIREMENTS.md` for the spec. Known gaps:

- **Sync runs once at boot.** `SYNC_INTERVAL_SECONDS` is parsed and validated but not
  yet acted on — photos added in Azure don't appear until restart (Phase 4.4).
- **Slideshow is not yet field-hardened**: all photos are loaded as textures up front,
  a corrupt image aborts the process, and an empty photo folder exits (Phase 5).
- Out of scope by design: first-boot Wi-Fi onboarding (balena handles it) and
  prev/next navigation buttons (advance is purely time-based).

## License
MIT — see `LICENSE.md`.
