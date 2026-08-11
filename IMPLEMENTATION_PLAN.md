# Digital Frame — Implementation Plan

Status as of **2026-08-11**. `REQUIREMENTS.md` is the spec; this file tracks *execution* —
what is built, what was deliberately dropped, and what is left.

**The frame is live.** A Pi 4 in the balena fleet `digiframe` has been booting, authenticating
to Azure, syncing, and rotating photos on a DSI panel since **2026-08-09**. The companion app
has been live since **2026-08-10**. Phases 0–6, 8 and 9 are done; what remains is test coverage,
CI hardening, and the second/third device.

## Scope (locked)
- **Fleet:** 2 devices, max 3. Balena, device type `raspberrypi4-64` (Raspberry Pi 4 Model B, aarch64).
  **1 deployed so far.**
- **Content:** ~20–30 photos at a time, rotating slideshow, time-based advance (no buttons).
- **Auth:** Entra service principal, read-only RBAC on one Blob container. Secrets via balena fleet vars.
- **Cost target:** pennies/month (see REQUIREMENTS.md §9). Diff-sync is a cost-safety mechanism.

## Test tooling
- **Unit tests** (`cargo test --workspace`) — pure logic in `core` (config, sync diff, name safety),
  plus the render-geometry and file-listing tests in `src/main.rs`. No network, no system libs for `core`.
- **Azurite** (Azure Storage emulator, Docker) — **not yet used.** This is the main outstanding gap.
- **Native `cargo run`** with SDL2 — real slideshow window on the dev machine.
- **Production** — the running frame now supersedes the one-off "real Azure smoke test".
- **buildx arm64 + QEMU** — `docker buildx build --platform linux/arm64` (slow: the Azure SDK's
  crypto stack compiles C under emulation).

---

# Done

## Phase 0 — Secret-leak containment ✅ (0.3 dropped)
- **0.1** ✅ The leaked SAS is dead: the old `benetmilian` account was **deleted**, which invalidates
  every SAS it ever signed. No key rotation was needed.
- **0.2** ✅ Hardcoded secrets gone from the working tree; `docker-compose.yaml` declares no
  credentials at all (balena injects them — see the comment there on why valueless pass-throughs
  are worse than nothing).
- **0.3** ⛔ **Dropped, deliberately.** Purging the token from git history buys nothing: it was
  container-scoped, read-only, already expired, and the account behind it no longer exists.
  Rewriting history has a real cost and this had no residual risk. Not debt — a decision.
- **0.4** ✅ `.env.example` with placeholders; `.env` gitignored.

## Phase 1 — Azure provisioning ✅
Storage account **`marcdigital`** (Standard/Hot/LRS), container **`photos`**, public access disabled.
Entra app registration → service principal, **single tenant**, **no redirect URI** (client-credentials
daemon flow, no interactive login). **Storage Blob Data Reader** assigned to the SP.

Two things worth remembering, both of which cost time:
- `az login --service-principal` prints **"No subscriptions found"** and that is *correct* — the SP
  intentionally holds only a data-plane role. The login still succeeds.
- The az CLI is awkward about subscription-less identities and kept falling back to the signed-in
  user, producing a misleading permission error. **The verification was abandoned as unnecessary:
  the frame authenticates with this SP in production, which proves the RBAC far better than the CLI
  ever could.** Don't re-litigate this if it comes up again.

⚠️ **Unverified:** whether *"Allow storage account key access"* was actually set to Disabled.
Check it — with two working service principals there is no reason to leave shared keys enabled.

## Phase 2 — Testable module refactor ✅
Workspace split: `core` holds pure logic and depends on neither SDL2 nor the Azure SDK, so
`cargo test -p marcdigital-core` runs anywhere. `core/src/store.rs` defines the `PhotoStore` trait
the sync logic is written against; `src/store.rs` is the Azure implementation.

## Phase 3 — Azure auth + sync ⚠️ (3.1 done, 3.2/3.3 outstanding)
- **3.1** ✅ `ClientSecretCredential` + blob container client, no SAS in the URL, SDK refreshes tokens.
- **3.2** ❌ **Azurite integration tests — not written.** See *Remaining*.
- **3.3** ✅ Superseded: the frame does this continuously in production.

## Phase 4 — Sync robustness ✅
Atomic downloads (`name.tmp` + rename), per-blob failure isolation, blob-name validation
(`safe_name()`), periodic background re-sync. **No manifest file** — the folder contents *are*
the manifest, so there is no second source of truth to drift. The sync task starts *after* the
display opens, so a frame booting before Wi-Fi shows its cached photos immediately instead of
sitting on a black panel for the network timeout.

## Phase 5 — Slideshow robustness ✅ (+ EXIF, unplanned)
Downscale-on-load to the panel, one texture held at a time, corrupt files skipped *and remembered*
(with mtime, so a re-upload gets a fresh chance), non-blocking event loop, waiting screen when empty,
folder re-scan every 10s, retry-instead-of-exit when the display is not ready, cursor hidden.

**Added beyond plan: EXIF orientation** (`kamadak-exif`). SDL2_image ignores the tag, so photos
straight off a phone camera rendered sideways — while the same photo sent via WhatsApp looked fine,
because WhatsApp bakes the rotation in and strips the tag. Panel rotation (`DISPLAY_ROTATION`) and
per-photo EXIF rotation compose.

Note on framing: the render path **fits** (contain), never crops — a 4:3 photo on a 16:9 panel is
letterboxed by design. A `cover`/crop mode would be a new option, not a bug fix.

## Phase 6 — aarch64 container + balena compose ✅
Native arm64 build on balena's builders; the `raspberrypi/tools` cross hack and the armv6
`.cargo/config.toml` block are both gone. **Mesa is required at runtime** even for software
rendering (SDL's KMSDRM backend builds its surface through GBM/EGL).

Lighting a DSI panel turned out to be four independent layers, each failing with a misleading
error on its own: device-tree overlay (**balena Configuration tab, not Variables**) → Mesa in the
image → `opengles2` renderer (desktop `opengl` reports `accelerated: true` and then never scans
out — a perfect render onto a black panel) → present every frame (`present()` page-flips, so
drawing once leaves the image in a back buffer). Full write-up in the README's *Display setup*.

## Phase 8 — Hardware deployment ⚠️ (one device)
- **8.1–8.4** ✅ Fleet `digiframe` created, fleet vars set, device joined, releases pushed, panel renders.
- **8.5** ⚠️ Partly: photos added/removed in Azure do appear/disappear. Reboot recovery and network-drop
  behaviour have not been deliberately exercised.
- **8.6** ❌ Second device not yet flashed.

**Wi-Fi provisioning — unplanned work, and a corrected assumption.** The spec originally said
"balena handles Wi-Fi". **It does not** — balenaOS ships no captive portal, and a frame handed to a
relative could never have joined their network (no keyboard, no shell, no input on the slideshow).
The fleet now runs **`wifi-connect` as a second service**, built from upstream source rather than
the balenaLabs block, whose published image dates from 2020 and dies on current balenaOS with
`RsnFlags property failed: wrong property type`.

## Phase 9 — Companion app ✅ live 2026-08-10
Repo **DigitalFrameApp**, Azure Static Web Apps (Free), installable PWA, `family` role by invitation.
Its README is authoritative; REQUIREMENTS.md §9b records only the shared contract. The critical
coupling: **the container is the entire interface between the two projects**, and each end
independently implements the same filename rule — `safe_name()` in `core/src/sync.rs` and
`safeName()` in the app's `api/shared/blob.js`. **Change one and you must change the other.**
Two service principals, not one: the frame reads, the app contributes.

## Small fixes + edition 2024 ✅ 2026-08-11
- **The video-driver probe no longer writes the environment.** `std::env::set_var` raced every
  other thread's `getenv` (the sync task and the Azure SDK are live by then) — UB in glibc. Now
  `sdl2::hint::set_with_priority("SDL_VIDEODRIVER", driver, &Hint::Override)`. **Override is
  load-bearing:** `SDL_GetHint` prefers the environment over a normal-priority hint, and
  `docker-compose.yaml` sets `SDL_VIDEODRIVER=kmsdrm`, so at normal priority the fallback loop
  would silently do nothing after the first attempt.
- **The success log reports what SDL actually chose** (`current_video_driver()`), not the
  candidate we asked for — so a build that ignored the hint says so instead of hiding it.
- **`Dockerfile` builder pinned to `--platform=linux/arm64`.** No-op on balena's builders; stops
  an x86 `docker compose up --build` from producing an image that only fails on the device.
- **Both crates moved to edition 2024** (floor Rust 1.85; toolchains are all ≥1.97). Fallout was
  small and entirely mechanical: `cargo fix --edition` added `+ use<>` to a test helper in
  `core/src/config.rs` (2024 makes RPIT capture all in-scope lifetimes), `cargo fmt` reordered
  imports under the 2024 style edition, and clippy gained a `collapsible_if` finding now that
  let-chains are stable. One advisory `tail-expr-drop-order` warning at `core/src/sync.rs:155`
  was reviewed and dismissed — the only destructor involved is `anyhow::Error`'s, which just
  frees memory.

---

# Remaining

## R1 — Azurite integration tests for `src/store.rs` 🔴 the real gap
Verified: **neither `src/store.rs` nor `core/src/store.rs` contains a single test.** The Azure client
is the only component checked purely by hand, and it is the one that touches money — a regression in
list/download is either a black frame or a surprise egress bill.

- Start Azurite as a Docker service; build the store against its connection-string credential.
- Seed blobs → `list()` returns exactly them → `download()` returns the correct bytes.
- Cover the failure paths that matter in the field: a blob deleted mid-sync, a network error
  (no crash, existing photos preserved), an unsafe blob name rejected.
- **Done when:** `src/store.rs` has real coverage and the suite runs in CI.

## R2 — CI hardening (finishes Phase 7)
`main.yml` currently runs fmt → clippy → test → build on `ubuntu-latest`, and `release.yml`
reuses it via `workflow_call` to gate the balena deploy. Missing:
- **Azurite service container** so R1's tests actually run in CI.
- **arm64 image build** (`docker buildx`) — nothing currently catches a container regression before
  it reaches balena.
- **Secret scanning** (gitleaks), scoped to new commits — history is knowingly dirty and is staying
  that way (see 0.3), so a full-history scan would fail forever and get ignored.

## R3 — Second device (Phase 8.6) + reboot/network drills (8.5)
Flash a second SD card, join the fleet, confirm it needs **zero per-device setup** — that is the
whole claim of the shared-fleet-auth design and it is currently unproven. Then deliberately exercise
reboot recovery and a network drop on both.

## Not doing
- **Git-history purge** (0.3) — see above.
- **Renaming the crate off `MarcDigital`** — build-internal only, never user-visible; the hotspot
  SSID `MarcDigital Setup` is kept because it is liked. Settled 2026-08-10, see REQUIREMENTS.md §6.
- **Nav buttons / GPIO** — out of scope, advance is purely time-based.
- **Masked secrets in balena** — no such feature exists. Mitigated by least privilege rather than
  solved; the token broker in REQUIREMENTS.md §9 is the real fix if it ever matters.

---

## Progress checklist
- [x] Phase 0 — secret containment *(0.3 dropped by decision)*
- [x] Phase 1 — Azure provisioning *(confirm shared-key access is disabled)*
- [x] Phase 2 — testable module refactor
- [~] Phase 3 — auth + sync *(3.1 done; 3.2 Azurite tests outstanding → R1)*
- [x] Phase 4 — sync robustness
- [x] Phase 5 — slideshow robustness + memory *(+ EXIF)*
- [x] Phase 6 — aarch64 container
- [~] Phase 7 — CI *(fmt/clippy/test/build + release; Azurite, arm64, gitleaks outstanding → R2)*
- [~] Phase 8 — hardware *(1 of 2–3 devices; wi-fi provisioning added → R3)*
- [x] Phase 9 — companion app — **live**

**Next up:** R1 — Azurite tests for the Azure client.
