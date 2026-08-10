# MarcDigital — Requirements & Project Spec

Authoritative spec for the project. Update this file when decisions change so it never has to be re-explained.

## 1. What it is
A **digital photo frame** for older family members. Relatives push photos to cloud storage; the frame syncs them and displays a rotating slideshow. Runs on a Raspberry Pi, fully automated (boots straight into the slideshow, no desktop/file access).

## 2. Language & stack
- **Entirely in Rust.** Any Python in the tree is legacy and has been removed.
- Rendering: **SDL2** (`sdl2` crate, image + static-link features), fullscreen slideshow.
- Async runtime: **tokio**.
- Cloud: **Azure Blob Storage** (`azure_storage_blob`, `azure_identity`, `azure_core`).
- Deployment: **Balena** (containers pushed as fleet updates). See Open Issues.

## 3. Functional requirements
1. **Wi-Fi onboarding — a `wifi-connect` service, not app code.**
   Corrected 2026-08-10: balenaOS does **not** ship a captive portal. The earlier
   assumption that "Balena handles it" was wrong, and a frame delivered to a relative
   could never have joined their network — no keyboard, no shell, no input on the
   slideshow. The fleet therefore runs the **balenaLabs `wifi-connect` block as a second
   service** in `docker-compose.yaml`: when the device has no connectivity it raises its
   own hotspot (`MarcDigital Setup`) with a captive portal, a phone joins it and picks the
   house network, and the credentials are saved to NetworkManager. The slideshow service is
   independent and keeps showing cached photos throughout. The app itself still implements
   no Wi-Fi UI.
2. **Cloud photo storage + local sync.** Master copy of images lives in **Azure Blob Storage**; the frame keeps a **local copy synced periodically** (`SYNC_INTERVAL_SECONDS`, default 30 min, in a background task started *after* the display opens). Sync is diff-based: download new blobs, delete local files no longer in the cloud. The folder contents *are* the manifest — there is no separate state file. Downloads are atomic (`name.tmp` + rename); blob names that are not plain filenames are rejected; a single blob failure does not abort the rest.
3. **Slideshow — purely time-based.** Fullscreen, aspect-ratio-preserving. Advance to the next image every X seconds. **No navigation buttons, no GPIO.**

## 4. Non-functional requirements
- Fully unattended: auto-start on boot, no way to reach the filesystem or a shell from the frame UI.
- Survives reboots and network drops (restart: always under Balena/compose).
- Secrets (Azure account/key/SAS, container name) come from **environment variables**, never hardcoded or committed.

## 5. Target hardware — DECIDED: Raspberry Pi 4 Model B (aarch64)
Target is the **Raspberry Pi 4 Model B** (aarch64, Cortex-A72), balena device type `raspberrypi4-64`.
Fully supported by balena with OTA. Plenty of RAM (1–8 GB) and CPU headroom vs. the Zero class.

History: the original Pi Zero (armv6) was ruled out — **balena discontinued that device type on
2026-03-01** (no OTA/OS updates) and armv6 is sunsetting ecosystem-wide (the armv6 cross build failed on
SDL2's transitive deps: the `raspberrypi/tools` sysroot didn't share the Debian `armhf` libs, plus an
armv6-vs-armv7 mismatch). The Zero 2 W was the interim pick; we moved up to the Pi 4 for the extra
power. Both are aarch64, so the native-build plan is unchanged — only the balena device type differs.

## 6. Current implementation status
Running on real hardware (Pi 4, balena fleet `digiframe`) since 2026-08-09.

- ✅ Azure Blob sync (`core/src/sync.rs` + `src/store.rs`): Entra service-principal auth,
  diff-based, atomic downloads, name validation, per-blob error recovery, periodic.
- ✅ SDL2 fullscreen slideshow (`src/main.rs`): one texture at a time downscaled to the
  panel, corrupt files skipped and remembered, waiting screen when empty, `DISPLAY_ROTATION`
  for landscape/portrait mismatch, cursor hidden, retries instead of exiting when the
  display is not ready.
- ✅ aarch64 Dockerfile + balena compose (the armv6 cross-compile hack is gone).
- ✅ CI (fmt/clippy/test/build) and tag-driven release to balena + GitHub Releases.
- ✅ Wi-Fi provisioning via the `wifi-connect` block (see §3.1).
- 🚫 Nav buttons / GPIO — out of scope (time-based only).
- ⏳ Companion app (§9b) — not started; today only the maintainer can add photos.
- ⏳ Azurite integration tests — `src/store.rs` still has no test coverage.

## 7. Known gaps / tech debt
Resolved: the leaked SAS (the `benetmilian` account was deleted, so it cannot be used —
the git-history purge was dropped as pointless); sync-once-at-boot; the `.unwrap()`s in the
render path; the split async model; Python-only CI.

Open:
- **`src/store.rs` has no tests.** The Azure client is the only component verified purely by
  hand. Azurite integration tests are the fix (§ Implementation Plan 3.2).
- **`std::env::set_var` runs inside the multi-threaded tokio runtime** (`src/main.rs`,
  video-driver selection). Unsound in principle, and a hard error on edition 2024.
- **Dockerfile builder stage has no `--platform`.** Correct on balena's arm64 builders;
  `docker compose up --build` on an x86 dev machine silently produces an unusable image.
- **Secrets are plaintext in the balena dashboard.** No masked-variable feature exists.
  Mitigated by least privilege (read-only, one container) rather than solved; the token
  broker in §9 is the real fix if it ever matters.
- **No secret scanning in CI.** Worth adding scoped to new commits.

## 8. Build plan (aarch64) — DONE
- Dockerfile builds natively for **aarch64** on balena's builders; the `raspberrypi/tools`
  cross-compile hack and the armv6 `.cargo/config.toml` block are both removed.
- SDL2 installed from apt, linked dynamically.
- **Mesa is required at runtime** (`libgl1-mesa-dri`, `libegl-mesa0`, `libgles2`, `libgbm1`,
  `libdrm2`): SDL's KMSDRM backend builds its surface through GBM/EGL even when rendering in
  software. See the README's *Display setup* section for the full four-layer chain
  (overlay → Mesa → `opengles2` → present-every-frame), each of which fails with a
  misleading error on its own.
- Keep dynamic SDL2 linking (no `static-link`).

## 9. Azure access & secrets — target design
- **Auth:** Microsoft Entra ID **service principal** (app registration) with **Storage Blob Data
  Reader** RBAC scoped to the single container. Use `azure_identity` (`ClientSecretCredential` /
  env credential); the SDK auto-refreshes short-lived tokens. *No* long-lived SAS baked into the device.
  (Managed Identity is not an option — the Pi is not Azure-hosted.)
- **Secret delivery:** inject `AZURE_*` as **balena fleet/device service variables**, never in git,
  code defaults, or `docker-compose.yaml`. Local dev uses a gitignored `.env`.
- **Least privilege:** read-only, single container (for the frames).
- **Storage tier: Hot.** At ~20–30 small photos, storage-at-rest is fractions of a cent, so Cool/Cold
  save nothing but add per-read costs and 30/90-day minimum-retention early-deletion fees — bad for an
  add/delete workflow. Archive is disqualified (not instant-access). Standard / Hot / LRS.

## 9b. Companion app — Azure Static Web Apps (Free)
Family curates photos (view / add / delete) from phones. **Not** a native app and **not** store-published.
- **Azure Static Web Apps Free tier** = one deployment bundling: PWA frontend + built-in auth +
  managed Functions API. Works on iPhone (Safari) and Android (Chrome) as an installable PWA — just a URL,
  no App Store / Play Store.
- **Auth:** SWA built-in providers. **On the Free plan only GitHub and Microsoft Entra ID are available**
  — Google (or any provider) needs custom OIDC, which is a **Standard-plan** feature (~$9/mo) and would
  break "nearly free", so we do **not** use it. Family logs in with **Microsoft accounts** (Entra also
  accepts personal Outlook/Hotmail/Live accounts — no work account needed) or GitHub. Invite family
  (free tier: 25 invited custom-role users), assign a `family` role, restrict all routes + API via
  `staticwebapp.config.json`. No passwords to build or leak.
- **API:** managed Functions `list` / `upload` / `delete`; storage credential stays **server-side**
  (app settings or managed identity). Client never holds Azure creds.
- **Cost:** free tier (100 GB bandwidth, 1M function executions) dwarfs family usage.
- **Storage account:** new account **`marcdigital`** (the old `benetmilian` is being retired/deleted,
  which auto-revokes the leaked SAS). Container **`photos`**.
- **Cleanup owed:** secrets already removed from the working tree (done); leaked SAS dies with the old
  account; optional git-history purge remains. Fail fast when env is missing (implemented in `core::config`).
- **Optional hardening:** a small token-broker backend that mints short-lived user-delegation SAS,
  so the device holds only an app key — overkill for a family frame but the most secure option.
