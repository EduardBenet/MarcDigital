# MarcDigital

A digital photo frame for older family members: relatives push photos to the cloud, the
frame syncs them locally and shows a rotating fullscreen slideshow. Runs on a Raspberry Pi,
fully automated. Written entirely in **Rust**.

> **See [`REQUIREMENTS.md`](./REQUIREMENTS.md) for the authoritative project spec** —
> requirements, target hardware, status, and known gaps.

## Stack
- **Rust** + **SDL2** for the fullscreen slideshow (`src/main.rs`)
- **Azure Blob Storage** for the photo master copy, synced locally (`src/fetcher.rs`)
- **tokio** async runtime
- **Balena** for deployment / fleet updates

## Configuration
All secrets come from environment variables (see `docker-compose.yaml`):

| Variable | Meaning |
|---|---|
| `AZURE_STORAGE_ACCOUNT` | Azure storage account name |
| `AZURE_STORAGE_KEY` | SAS token / key (do **not** commit) |
| `CONTAINER_NAME` | Blob container holding the photos |

Photos are synced into `./synced_photos` with a `manifest.txt` tracking local state.

## Build & run
Local build:
```
cargo build --release
```

Cross-compile for the Pi (armv6) and run via Balena/compose:
```
docker compose up --build
```

## Status
Slideshow and Azure sync work. Not yet implemented: first-boot Wi-Fi onboarding screen,
prev/next navigation buttons, periodic re-sync. See `REQUIREMENTS.md`.

## License
MIT — see `LICENSE.md`.
