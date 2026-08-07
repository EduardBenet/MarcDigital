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
1. **First-boot Wi-Fi onboarding — handled by Balena, not app code.** Balena provides Wi-Fi provisioning (captive portal / `wifi-connect`); the app does **not** implement a Wi-Fi screen. The app assumes connectivity is present.
2. **Cloud photo storage + local sync.** Master copy of images lives in **Azure Blob Storage**; the frame keeps a **local copy synced periodically**. Sync is diff-based: download new blobs, delete local files no longer in the cloud, track state in a manifest. *(Periodic re-sync deferred — currently syncs once at boot.)*
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
- ✅ Azure Blob sync (`src/fetcher.rs`): diff against manifest, download/delete, rewrite manifest.
- ✅ SDL2 fullscreen slideshow (`src/main.rs`): scales to fit, time-based rotation.
- ⚠️ Cross-compile Dockerfile (armv6) — **the current blocker**, see §8.
- 🚫 Wi-Fi screen — out of scope (Balena handles it).
- 🚫 Nav buttons / GPIO — out of scope (time-based only).
- ⏳ Periodic re-sync — deferred (syncs once at boot for now).
- ❌ Secrets hygiene (see gaps).

## 7. Known gaps / tech debt
- **Secrets committed to git**: a live-style SAS token appears in `src/main.rs` default and `docker-compose.yaml`. Even if expired, rotate and remove; load only from env. Consider `azure_identity` managed identity or a mounted secret.
- **Sync runs once at boot**, not periodically — needs a timer/loop (deferred).
- **No error resilience**: many `.unwrap()`s; a bad image or network blip can crash the frame.
- **`fetcher::sync_folder` uses its own `#[tokio::main]`** while `main` is sync — inconsistent async model; unify.
- **CI** was Python-only; replaced with a Rust build workflow.

## 8. Build plan (aarch64 / Zero 2 W)
- Rewrite the Dockerfile for **aarch64** — balena builds arm64 natively, so drop the whole
  `raspberrypi/tools` cross-compile hack. Build against a `balenalib/raspberrypi-zero-2-w-debian`
  (or generic `arm64` Debian) base; install SDL2 from apt (arm64 has proper packages).
- `.cargo/config.toml` armv6 target block becomes unnecessary for the balena build.
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
