# MarcDigital — Implementation Plan

Step-by-step plan to take the project from its current state to a tested, deployable
Raspberry Pi Zero 2 W photo-frame fleet. **Everything here is verifiable without the Pi**,
except Phase 8 (hardware). Each step lists what to do, how to test it, and its done-criteria.

## Scope (locked)
- **Fleet:** 2 devices, max 3. Balena, device type `raspberrypi4-64` (Raspberry Pi 4 Model B, aarch64).
- **Content:** ~20–30 photos at a time, rotating slideshow, time-based advance (no buttons).
- **Auth:** Entra ID service principal, read-only RBAC on one Blob container. Secrets via balena fleet vars.
- **Cost target:** pennies/month (see REQUIREMENTS.md §9). Diff-sync is a cost-safety mechanism.

## Test tooling (no hardware needed)
- **Unit tests** (`cargo test`) — pure logic (config parsing, sync diff). No network.
- **Azurite** (Azure Storage emulator, Docker) — integration tests for list/download/delete. Free, offline.
- **Native `cargo run`** on the dev machine with SDL2 — real slideshow window to verify rendering/timing/robustness.
- **Real Azure smoke test** — one run with the actual service principal to prove auth wiring (costs ~$0).
- **buildx arm64 + QEMU** — cross-build the container and run it headless (`SDL_VIDEODRIVER=dummy`).

---

## Phase 0 — Secret-leak containment (do FIRST)
The repo history contains a live-format SAS token (`src/main.rs`, `docker-compose.yaml`).

- **0.1** **Revoke the leaked SAS.** The old account (`benetmilian`) is being **retired** — deleting it
  invalidates every SAS signed by it, so no key rotation is needed. A brand-new account `marcdigital`
  replaces it. (If the old account were kept, the fallback would be rotating its access keys.)
- **0.2** Remove hardcoded secrets/defaults from `src/main.rs` and `docker-compose.yaml` (done properly in Phase 3/6).
- **0.3** Purge the secret from git history with `git filter-repo` (or BFG), then force-push. *(Destructive to history — confirm before running.)*
- **0.4** Add a `.env.example` (placeholders only) and ensure `.env` is gitignored.
- **Test:** `gitleaks detect` (or `git log -p -S 'sig='`) reports no secrets in history.
- **Done when:** old token is revoked in Azure AND absent from history; no secret in the working tree.

## Phase 1 — Azure provisioning (portal/CLI, no code)
- **1.1** Storage account **`marcdigital`** (new; replaces the retired `benetmilian`): **Standard, Hot,
  LRS**, single container **`photos`**. Public access **disabled**. After the service principal works,
  set **"Allow storage account key access" = Disabled** so only Entra RBAC remains.
- **1.2** Create an **Entra app registration** (service principal); generate a **client secret** (note expiry, ~24 mo).
- **1.3** Assign RBAC **Storage Blob Data Reader** to the SP, **scoped to the container** (least privilege, read-only).
- **1.4** Record the five values: `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
  `AZURE_STORAGE_ACCOUNT`, `CONTAINER_NAME`. Put them in local `.env` (gitignored).
- **Test (proves RBAC before any code):**
  ```
  az login --service-principal -u $AZURE_CLIENT_ID -p $AZURE_CLIENT_SECRET --tenant $AZURE_TENANT_ID
  az storage blob list --account-name $AZURE_STORAGE_ACCOUNT -c $CONTAINER_NAME --auth-mode login
  ```
  Listing succeeds (read) and an upload attempt is **denied** (confirms read-only).
- **Done when:** the SP can list/read the container and nothing else.

## Phase 2 — Refactor into testable modules
Restructure `src/` so logic is unit-testable and auth is swappable:
- **2.1** `config.rs` — typed `Config` loaded from env; **fails fast** with a clear error if any var is missing. No defaults.
- **2.2** `store.rs` — a `PhotoStore` trait (`list() -> Set<name>`, `download(name) -> bytes`) + an Azure impl.
  The trait lets tests use a fake store.
- **2.3** `sync.rs` — **pure** diff function `plan_sync(local: &Set, cloud: &Set) -> {to_download, to_delete}`
  and an orchestrator that calls the store + manifest I/O.
- **2.4** `slideshow.rs` — SDL rendering/loop, decoupled from sync.
- **2.5** `main.rs` — wires config → sync → slideshow.
- **Test:** unit tests for `config` (missing/valid env) and `plan_sync` (add-only, delete-only, mixed,
  empty manifest, no-change). These cover the **cost-critical** diff logic.
- **Done when:** `cargo test` green; modules compile with no behavior change yet.

## Phase 3 — Azure auth + sync implementation
- **3.1** Implement the Azure `PhotoStore` with `azure_identity::ClientSecretCredential` and
  `BlobContainerClient` built from `https://{account}.blob.core.windows.net` + credential + container.
  **No SAS-in-URL.** SDK auto-refreshes tokens.
- **3.2** Integration test the store against **Azurite** (started as a Docker service; tests use Azurite's
  connection-string credential): seed blobs → `list()` returns them → `download()` returns correct bytes.
- **3.3** **Real-Azure smoke test** (ignored-by-default test or a small binary): run with the actual SP env
  → confirms Entra auth + RBAC + endpoint wiring end-to-end.
- **Test:** Azurite integration passes in CI; smoke test passes locally against real Azure.
- **Done when:** the app authenticates with the SP and syncs the real container.

## Phase 4 — Sync robustness (field-hardening + cost safety)
- **4.1** **Atomic downloads:** write to `name.tmp` then rename to final (no half-written JPEGs on power loss).
- **4.2** **Retry with backoff** on transient network/HTTP errors; a failed sync must not crash — log and keep
  serving existing local photos.
- **4.3** Verify the **diff** only downloads new/changed blobs (guards the Azure bill).
- **4.4** **Periodic re-sync:** background task on a timer (configurable, default e.g. 30 min).
  *(Previously deferred; included here because it's core to a frame and to cost behavior.)*
- **Test:** integration tests against Azurite for: interrupted download leaves no corrupt file;
  add/remove blobs between syncs reflected locally; simulated network error → no crash, ret/recovers.
- **Done when:** repeated syncs are correct, atomic, crash-proof, and download only deltas.

## Phase 5 — Slideshow robustness + memory
- **5.1** **Downscale on load** to the display resolution — a single full-res phone photo decodes to ~48 MB as a
  texture; holding 30 at once wastes hundreds of MB. Less dire on a Pi 4 (1–8 GB) than the Zero class, but
  still the right design; never hold them all.
- **5.2** **Lazy loading:** keep only the current (and optionally next, preloaded) texture; free the rest.
- **5.3** **Skip corrupt/unsupported files** instead of crashing; log and continue.
- **5.4** **Non-blocking loop:** poll events on a short tick and advance by *elapsed time* (no 30 s `thread::sleep`
  that freezes input); handle quit promptly.
- **5.5** **Empty/degraded states:** if no photos yet, show a "connecting / no photos" screen instead of exiting.
- **5.6** Pick up new photos when a sync lands (re-scan folder), without restarting.
- **Test (native, real window on dev machine):** `cargo run` with a local folder — verify rotation timing,
  aspect-ratio scaling, a deliberately corrupt file is skipped, memory stays flat over many rotations,
  quit works instantly, empty-folder state shows.
- **Done when:** the slideshow runs indefinitely on the dev machine without leaking or crashing.

## Phase 6 — aarch64 container + Balena compose
- **6.1** **Rewrite the Dockerfile for arm64**: base `balenalib/raspberrypi4-64-debian:bookworm`,
  **native** build (balena builds arm64 on-device/on-builder — drop the `raspberrypi/tools` cross hack and the
  armv6 `.cargo/config.toml` block). Install SDL2 + SDL2_image from apt; runtime installs the shared libs.
- **6.2** **`docker-compose.yaml` for balena:** no secrets (fleet vars supply them); grant display/GPU access
  for the frame (KMSDRM: `/dev/dri`, udev, appropriate labels). `restart: always`.
- **6.3** Volume for `synced_photos` persisted across restarts.
- **Test:**
  - `docker buildx build --platform linux/arm64 ...` **builds and links** (proves the aarch64 toolchain/deps).
  - Run the image headless under QEMU with `SDL_VIDEODRIVER=dummy` + real SP env → it **completes a sync and
    enters the loop** (GUI rendering is deferred to hardware, but sync + control flow are verified in-container).
- **Done when:** arm64 image builds, and runs a headless sync successfully.

## Phase 7 — CI
- **7.1** GitHub Actions: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` (with an **Azurite service
  container** for integration tests).
- **7.2** Add the **arm64 `docker buildx` build** as a CI job (catches container regressions).
- **7.3** Add a **secret scan** (gitleaks) job.
- **Test:** CI is green on a clean push; a deliberately broken commit fails the right job.
- **Done when:** every push is fmt+lint+unit+integration+arm64-build+secret-scan verified.

## Phase 8 — Hardware deployment (the only untested-until-hardware part)
- **8.1** Create balena fleet (`raspberrypi4-64`); set the five `AZURE_*` vars as **fleet variables**.
- **8.2** Flash balenaOS to 2 SD cards, join devices to the fleet.
- **8.3** `balena push` the release; confirm both devices pull and run.
- **8.4** Configure the display path (KMSDRM/kiosk, HDMI/boot config as needed) and confirm the slideshow renders.
- **8.5** Validate on-device: photos rotate, a photo added/removed in Azure appears/disappears within a sync
  cycle, reboot recovers, Wi-Fi provisioning (balena) works on a fresh device.
- **8.6** Add the 3rd device by flashing + joining (verifies "shared fleet auth, zero per-device setup").
- **Done when:** 2–3 frames run unattended, sync correctly, and survive reboots/network drops.

## Phase 9 — Companion app (Azure Static Web Apps, Free)
A phone app for the family to view / add / delete photos. Independent of the frame (can be built in
parallel). PWA + built-in auth + managed API, one free SWA deployment. iPhone + Android, no app store.
- **9.1** Scaffold SWA: static PWA frontend (thumbnail grid + Add/Delete buttons, `manifest.json` +
  service worker for installability) and a managed **Functions API**: `GET /photos` (list), `POST /photos`
  (upload), `DELETE /photos/{name}`. API holds the storage credential **server-side** (app settings /
  managed identity) — never in the client.
- **9.2** **Auth + allowlist:** enable SWA built-in auth. **Free plan supports only GitHub and Microsoft
  Entra ID** — Google/others need custom OIDC = Standard plan (~$9/mo), so we stay on the built-ins
  (Microsoft accounts, incl. personal Outlook/Hotmail; or GitHub). Note: adding a custom provider disables
  the built-ins, so don't. Restrict all routes + API to `allowedRoles: ["family"]` in
  `staticwebapp.config.json`; invite family members (free tier: 25 invited custom-role users) and assign
  the `family` role.
- **9.3** Wire the API to the **same Hot container** the frames read; uploads land as blobs, deletes remove them.
- **Test (local, no deploy):** `swa start` emulator serves the app with **emulated auth + functions**;
  point the API at **Azurite** and verify: unauthenticated user is blocked, an authed `family` user can
  list/upload/delete, and changes show up in the (Azurite) container. Then a staging-environment smoke test
  against real Azure.
- **Done when:** an invited family member can add/delete photos from an iPhone and an Android browser,
  and those changes are what the frames later sync.

---

## Progress checklist
- [ ] Phase 0 — secret containment
- [ ] Phase 1 — Azure provisioning
- [ ] Phase 2 — testable module refactor
- [ ] Phase 3 — auth + sync implementation
- [ ] Phase 4 — sync robustness
- [ ] Phase 5 — slideshow robustness + memory
- [ ] Phase 6 — aarch64 container
- [ ] Phase 7 — CI
- [ ] Phase 8 — hardware deployment
- [ ] Phase 9 — companion app (Azure Static Web Apps) — parallelizable
