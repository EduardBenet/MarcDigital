//! MarcDigital frame: sync photos from Azure, then run a fullscreen slideshow.
//!
//! The frame is unattended — nobody can reach a shell to restart it — so the
//! slideshow is written to survive the things that actually happen in the
//! field: a corrupt JPEG, an empty photo folder on first boot, a folder whose
//! contents change underneath it, and a display that must stay responsive to a
//! quit request.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

extern crate sdl2;

use sdl2::event::{Event, WindowEvent};
use sdl2::image::LoadSurface;
use sdl2::keyboard::Keycode;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{Texture, TextureCreator};
use sdl2::surface::Surface;
use sdl2::video::WindowContext;

use marcdigital_core::config::Config;
use marcdigital_core::sync::run_sync;

mod store;
use store::AzureBlobStore;

/// Whether to emit verbose diagnostics.
///
/// The frame runs unattended for months, so anything logged per rotation would
/// swamp the device log (a 30s rotation is ~2900 lines/day). Everything needed
/// to debug the display path is therefore off by default and re-enabled with
/// the `MARCDIGITAL_VERBOSE` service variable. Warnings and errors are never
/// gated - only the routine narration is.
fn verbose() -> bool {
    static VERBOSE: OnceLock<bool> = OnceLock::new();
    *VERBOSE.get_or_init(|| {
        std::env::var("MARCDIGITAL_VERBOSE")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    })
}

/// `println!` that only fires under [`verbose`].
macro_rules! vlog {
    ($($arg:tt)*) => {
        if verbose() {
            println!($($arg)*);
        }
    };
}

/// How often the loop wakes to poll events. Short enough that a quit is acted
/// on immediately, long enough that the frame stays near-idle between redraws.
const TICK: Duration = Duration::from_millis(50);

/// How often the photo folder is re-read, so photos that land from a sync are
/// picked up without a restart. Reading ~30 directory entries is cheap.
const RESCAN_INTERVAL: Duration = Duration::from_secs(10);

/// Extensions we will attempt to decode. Anything else in the folder — a stray
/// text file, a partially written download — is ignored rather than being fed
/// to the decoder.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp", "gif", "webp", "tif", "tiff"];

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Files that failed to decode, with the modification time they had when they
/// failed. Keeping the timestamp means a file *replaced* under the same name
/// gets a fresh chance, while an unchanged bad file stays skipped.
type SkipList = HashMap<PathBuf, Option<SystemTime>>;

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Candidate image files, sorted so rotation order is stable across restarts
/// and re-scans. A folder we cannot read is logged and treated as empty — that
/// is a degraded state to display, not a reason to exit.
///
/// Entries in `skip` are filtered out here rather than after loading. Doing it
/// later meant the rescan kept resurrecting a known-bad file: the in-memory
/// list had dropped it, the on-disk listing had not, so the two never matched
/// and the frame re-decoded and re-logged the same failure every 10 seconds.
fn list_photos(dir: &Path, skip: &SkipList) -> Vec<PathBuf> {
    let mut photos: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && is_image(path))
            .filter(|path| match skip.get(path) {
                // Still the same bytes that failed before: leave it out.
                Some(failed_at) => *failed_at != mtime(path),
                None => true,
            })
            .collect(),
        Err(e) => {
            eprintln!("Could not read {}: {e}", dir.display());
            Vec::new()
        }
    };
    photos.sort();
    photos
}

/// Decode one image, downscaling it to at most `max_w` x `max_h` *before* it
/// becomes a GPU texture. A full-res phone photo is ~48 MB as a texture but a
/// display-sized one is a few MB, and the frame can never show more detail than
/// the panel has anyway.
fn load_scaled_texture<'a>(
    creator: &'a TextureCreator<WindowContext>,
    path: &Path,
    max_w: u32,
    max_h: u32,
) -> Result<Texture<'a>, String> {
    let surface = Surface::from_file(path)?;
    let (img_w, img_h) = (surface.width(), surface.height());
    if img_w == 0 || img_h == 0 {
        return Err("image has zero dimension".to_string());
    }

    // Only ever shrink: upscaling here would cost memory and add nothing, since
    // the renderer stretches to fit at draw time regardless.
    let scale = f32::min(max_w as f32 / img_w as f32, max_h as f32 / img_h as f32);
    if scale >= 1.0 {
        return creator
            .create_texture_from_surface(&surface)
            .map_err(|e| e.to_string());
    }

    // Normalise the pixel format first: `blit_scaled` between mismatched
    // formats (e.g. a palettised GIF) is where odd failures come from.
    let surface = surface.convert_format(PixelFormatEnum::RGBA32)?;
    let dst_w = ((img_w as f32 * scale) as u32).max(1);
    let dst_h = ((img_h as f32 * scale) as u32).max(1);
    let mut scaled = Surface::new(dst_w, dst_h, PixelFormatEnum::RGBA32)?;
    surface.blit_scaled(None, &mut scaled, None)?;

    creator
        .create_texture_from_surface(&scaled)
        .map_err(|e| e.to_string())
}

/// Load the photo at `index`, skipping past any file that fails to decode.
///
/// A corrupt or truncated image is dropped from `photos`, recorded in `skip`
/// and logged once, so the frame moves on instead of dying. Returns `None` only
/// when nothing in the list could be decoded at all.
fn load_current<'a>(
    creator: &'a TextureCreator<WindowContext>,
    photos: &mut Vec<PathBuf>,
    index: &mut usize,
    skip: &mut SkipList,
    max_w: u32,
    max_h: u32,
) -> Option<Texture<'a>> {
    while !photos.is_empty() {
        if *index >= photos.len() {
            *index = 0;
        }
        match load_scaled_texture(creator, &photos[*index], max_w, max_h) {
            Ok(texture) => return Some(texture),
            Err(e) => {
                // Skipping is deliberate: one bad file must not take the frame
                // down. It stays on disk (the next sync owns deleting it) but
                // is remembered so later rescans do not retry it forever.
                let bad = photos.remove(*index);
                eprintln!("Skipping {}: {e}", bad.display());
                let stamp = mtime(&bad);
                skip.insert(bad, stamp);
            }
        }
    }
    None
}

/// The "no photos yet" screen: shown on first boot before the first sync lands,
/// or if every file failed to decode. Deliberately font-free (no SDL_ttf
/// dependency and no font to ship) — three dim blocks on a dark background,
/// which reads as "waiting" rather than as a broken display.
fn draw_waiting_screen(canvas: &mut sdl2::render::WindowCanvas) -> Result<(), String> {
    canvas.set_draw_color(Color::RGB(16, 16, 20));
    canvas.clear();

    let (w, h) = canvas.output_size()?;
    let block = (w / 60).clamp(6, 24);
    let gap = block * 2;
    let total = block * 3 + gap * 2;
    let x0 = (w.saturating_sub(total)) / 2;
    let y = h / 2;

    canvas.set_draw_color(Color::RGB(70, 70, 80));
    for i in 0..3 {
        let x = x0 + i * (block + gap);
        canvas.fill_rect(Rect::new(x as i32, y as i32, block, block))?;
    }
    Ok(())
}

/// Report what the container can actually see of the display hardware.
///
/// The frame is headless and unattended, so when the display fails to come up
/// the device log is the only evidence available. Printing this unconditionally
/// costs nothing at boot and turns "kmsdrm not available" from a dead end into
/// something diagnosable without another push.
fn log_display_environment() {
    match std::fs::read_dir("/dev/dri") {
        Ok(entries) => {
            let mut nodes: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            nodes.sort();
            if nodes.is_empty() {
                println!("/dev/dri exists but is empty (no card/render nodes in this container)");
            } else {
                vlog!("/dev/dri contains: {}", nodes.join(", "));
            }
        }
        // Not gated: no DRM nodes means the display can never work.
        Err(e) => println!("/dev/dri not visible from this container: {e}"),
    }

    // SDL's KMSDRM backend opens a VT to take over the console; without one it
    // can fail even when the DRM nodes are present.
    let ttys: Vec<&str> = ["/dev/tty0", "/dev/tty", "/dev/console"]
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .collect();
    vlog!(
        "console devices: {}",
        if ttys.is_empty() {
            "none visible".to_string()
        } else {
            ttys.join(", ")
        }
    );

    vlog!(
        "SDL video drivers compiled in: {}",
        sdl2::video::drivers().collect::<Vec<_>>().join(", ")
    );
}

/// Bring up the video subsystem, trying each candidate driver in turn.
///
/// A frame that exits because one backend was unavailable just crash-loops
/// under `restart: always`, so we fall back rather than give up: whichever
/// driver works, the slideshow runs. `SDL_VIDEODRIVER` (set in the compose
/// file) is honoured first when present, then the sensible defaults for a Pi.
fn init_video(sdl: &sdl2::Sdl) -> Result<sdl2::VideoSubsystem, String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(preferred) = std::env::var("SDL_VIDEODRIVER") {
        if !preferred.trim().is_empty() {
            candidates.push(preferred.trim().to_string());
        }
    }
    for fallback in ["kmsdrm", "x11", "wayland"] {
        if !candidates.iter().any(|c| c == fallback) {
            candidates.push(fallback.to_string());
        }
    }

    let mut errors = Vec::new();
    for driver in &candidates {
        // SDL reads this at subsystem-init time, so it must be set per attempt.
        std::env::set_var("SDL_VIDEODRIVER", driver);
        match sdl.video() {
            Ok(video) => {
                println!("Video driver in use: {driver}");
                return Ok(video);
            }
            Err(e) => {
                eprintln!("Video driver {driver} unavailable: {e}");
                errors.push(format!("{driver}: {e}"));
            }
        }
    }

    // ASCII only: the balena log pipeline mangles non-ASCII punctuation.
    Err(format!(
        "no usable video driver. Tried: {}",
        errors.join("; ")
    ))
}

/// How long to wait before re-attempting a failed video init.
const VIDEO_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Pick the render driver, preferring GLES2.
///
/// Left to itself SDL takes the first driver in its list, which is desktop
/// `opengl`. The Pi's VideoCore does not do desktop GL, so that resolves to a
/// Mesa path that creates a context reporting `accelerated: true` and then
/// never scans out - the app renders perfectly onto a black panel. `opengles2`
/// is the backend that actually works here.
///
/// Returns `None` (meaning "let SDL decide") when GLES2 is not among the
/// compiled-in drivers, or when `SDL_RENDER_DRIVER` is set so a human can still
/// override this from the environment.
fn preferred_render_driver() -> Option<u32> {
    if std::env::var("SDL_RENDER_DRIVER").is_ok_and(|v| !v.trim().is_empty()) {
        vlog!("SDL_RENDER_DRIVER is set; leaving the choice to SDL.");
        return None;
    }

    let drivers: Vec<String> = sdl2::render::drivers()
        .map(|d| d.name.to_string())
        .collect();
    vlog!("Render drivers available: {}", drivers.join(", "));

    match drivers.iter().position(|name| name == "opengles2") {
        Some(index) => Some(index as u32),
        None => {
            eprintln!("opengles2 not available; falling back to SDL's default choice.");
            None
        }
    }
}

/// Build the fullscreen window and its canvas.
///
/// `driver_index` selects a specific SDL render driver; `None` means index -1,
/// where SDL walks its whole driver list and settles on the first that works.
/// The window is created here rather than passed in because `into_canvas`
/// consumes it, so a failed canvas build needs a fresh window to retry with.
fn build_canvas(
    video: &sdl2::VideoSubsystem,
    w: u32,
    h: u32,
    driver_index: Option<u32>,
) -> Result<sdl2::render::WindowCanvas, String> {
    let window = video
        .window("Slideshow", w, h)
        .fullscreen_desktop()
        .build()
        .map_err(|e| format!("creating window: {e}"))?;

    let mut builder = window.into_canvas();
    if let Some(index) = driver_index {
        builder = builder.index(index);
    }
    builder.build().map_err(|e| format!("creating canvas: {e}"))
}

/// Open the display: video subsystem, fullscreen window, and canvas.
///
/// Every step here can fail on a frame whose panel is not ready, so they are
/// kept together and retried as one unit. Splitting them means a later step
/// escaping the retry, which is exactly how an "EGL not initialized" at window
/// creation took the whole container down.
///
/// The window deliberately adopts the panel's *current* mode rather than
/// requesting a size: under KMSDRM a fixed request is a real modeset, and an
/// 800x480 panel simply has no 1920x1080 mode to switch to.
fn open_display(
    sdl: &sdl2::Sdl,
) -> Result<(sdl2::VideoSubsystem, sdl2::render::WindowCanvas), String> {
    let video = init_video(sdl)?;

    // Enumerate every display SDL sees. On a Pi the DSI panel and two HDMI
    // connectors can all be candidates, and picking the wrong index yields a
    // window that renders correctly to a screen nobody is looking at.
    match video.num_video_displays() {
        Ok(n) => {
            vlog!("SDL reports {n} display(s)");
            for i in 0..n {
                let name = video
                    .display_name(i)
                    .unwrap_or_else(|_| "<unnamed>".to_string());
                match video.display_bounds(i) {
                    Ok(b) => vlog!("  display {i}: {name} {}x{}", b.width(), b.height()),
                    Err(e) => vlog!("  display {i}: {name} (bounds unavailable: {e})"),
                }
            }
        }
        Err(e) => eprintln!("Could not enumerate displays: {e}"),
    }

    let (w, h) = match video.display_bounds(0) {
        Ok(bounds) => (bounds.width(), bounds.height()),
        Err(e) => {
            // Not fatal: fullscreen_desktop ignores the requested size anyway,
            // so any plausible value gets us to a window.
            eprintln!("Could not query display bounds ({e}); falling back to 1280x720");
            (1280, 720)
        }
    };

    // Prefer GLES2, but never let that preference become a hard requirement:
    // asking SDL for one specific driver disables its own fallback chain, so a
    // device where opengles2 is compiled in yet fails to create a context would
    // otherwise retry forever instead of settling for a driver that works.
    let canvas = match preferred_render_driver() {
        Some(index) => match build_canvas(&video, w, h, Some(index)) {
            Ok(canvas) => canvas,
            Err(e) => {
                eprintln!("Preferred renderer (opengles2) failed: {e}. Letting SDL choose.");
                build_canvas(&video, w, h, None)?
            }
        },
        None => build_canvas(&video, w, h, None)?,
    };

    let info = canvas.info();
    let accelerated =
        info.flags & (sdl2::sys::SDL_RendererFlags::SDL_RENDERER_ACCELERATED as u32) != 0;
    println!(
        "Renderer in use: {} (accelerated: {accelerated})",
        info.name
    );

    Ok((video, canvas))
}

/// Wait for the display, rather than giving up on it.
///
/// Exiting would be wrong twice over: under `restart: always` it becomes a
/// crash loop that also destroys the container you need a shell in to debug,
/// and in the field a frame whose TV is simply switched off at boot would never
/// recover. Retrying costs nothing and self-heals when the display appears.
fn wait_for_display(sdl: &sdl2::Sdl) -> (sdl2::VideoSubsystem, sdl2::render::WindowCanvas) {
    let mut attempt: u32 = 0;
    loop {
        match open_display(sdl) {
            Ok(display) => return display,
            Err(e) => {
                attempt += 1;
                eprintln!(
                    "No display yet (attempt {attempt}): {e}. \
                     Retrying in {}s; the frame stays running.",
                    VIDEO_RETRY_INTERVAL.as_secs()
                );
                std::thread::sleep(VIDEO_RETRY_INTERVAL);
            }
        }
    }
}

/// Run one sync and report it, keeping the routine case quiet.
///
/// Never returns an error: a sync failure is survivable because the frame still
/// has whatever is already on disk, and the next tick retries.
async fn sync_once(store: &AzureBlobStore, photo_dir: &Path) {
    match run_sync(store, photo_dir).await {
        Ok(plan) => {
            if plan.is_clean_noop() {
                vlog!("Sync: no changes.");
            } else {
                println!(
                    "Sync: {} downloaded, {} deleted.",
                    plan.to_download.len(),
                    plan.to_delete.len()
                );
            }
            for name in &plan.rejected {
                eprintln!("Ignored unsafe blob name {name:?} (not a plain filename).");
            }
            for (name, reason) in &plan.failed {
                eprintln!("Sync problem with {name}: {reason}");
            }
        }
        Err(e) => eprintln!("Sync failed (keeping existing photos): {e:#}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::from_env()?;

    let photo_store = Arc::new(AzureBlobStore::new(
        &config.tenant_id,
        &config.client_id,
        &config.client_secret,
        &config.storage_account,
        &config.container,
    )?);

    // Sync runs in the background, on a timer, and deliberately *after* the
    // display is opened below. Doing it inline at startup meant a frame booting
    // before Wi-Fi came up sat on a black panel for the whole network timeout -
    // with the waiting screen, built for exactly that case, unreachable until
    // the sync returned.
    {
        let store = Arc::clone(&photo_store);
        let photo_dir = config.photo_dir.clone();
        let interval = config.sync_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // `interval` fires immediately on the first tick, so the initial
            // sync still happens at startup - just without blocking the screen.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                sync_once(&store, &photo_dir).await;
            }
        });
    }
    println!(
        "Syncing {}/{} every {}s.",
        config.storage_account,
        config.container,
        config.sync_interval.as_secs()
    );

    let sdl_context = sdl2::init()?;
    log_display_environment();
    // Held for the lifetime of the program: dropping the video subsystem would
    // tear down the window it created.
    let (_video, mut canvas) = wait_for_display(&sdl_context);

    // No pointer on a photo frame. KMSDRM draws a cursor by default even with
    // no mouse attached, and there is no way to dismiss it on a finished frame.
    sdl_context.mouse().show_cursor(false);

    let texture_creator = canvas.texture_creator();

    // Decode target: the panel's own resolution, so textures are never larger
    // than what can actually be shown.
    let (mut screen_w, mut screen_h) = canvas.output_size()?;
    vlog!("Canvas output size: {screen_w}x{screen_h}");

    let mut skip: SkipList = SkipList::new();
    let mut photos = list_photos(&config.photo_dir, &skip);
    // Logged unconditionally: "how many photos did it actually find, and where"
    // is the first question asked whenever the panel looks wrong.
    println!(
        "Found {} photo(s) in {}",
        photos.len(),
        config.photo_dir.display()
    );
    if photos.is_empty() {
        // Not an error, and explicitly not an exit: the first sync may not have
        // landed yet, and under `restart: always` exiting here is a crash loop.
        println!("No photos yet; showing the waiting screen.");
    }

    let mut index = 0usize;
    let mut current: Option<Texture> = None;
    let mut needs_load = true;
    let mut last_advance = Instant::now();
    let mut last_rescan = Instant::now();

    let mut event_pump = sdl_context.event_pump()?;

    'main: loop {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'main,
                Event::Window {
                    win_event: WindowEvent::SizeChanged(..) | WindowEvent::Exposed,
                    ..
                } => {
                    let (w, h) = canvas.output_size()?;
                    if (w, h) != (screen_w, screen_h) {
                        // Panel geometry changed, so the cached texture was
                        // decoded for the wrong size.
                        screen_w = w;
                        screen_h = h;
                        needs_load = true;
                    }
                }
                _ => {}
            }
        }

        // Pick up photos a sync has added or removed, without a restart.
        if last_rescan.elapsed() >= RESCAN_INTERVAL {
            last_rescan = Instant::now();
            let found = list_photos(&config.photo_dir, &skip);
            if found != photos {
                // Stay on the photo currently displayed if it survived, so a
                // sync does not visibly jolt the rotation.
                let showing = photos.get(index).cloned();
                photos = found;
                index = showing
                    .and_then(|path| photos.iter().position(|p| *p == path))
                    .unwrap_or(0);
                // A photo arriving mid-interval should still get its full turn.
                last_advance = Instant::now();
                needs_load = true;
            }
        }

        // Time-based advance, measured from the clock rather than by sleeping
        // through the interval — so events stay responsive the whole time.
        if photos.len() > 1 && last_advance.elapsed() >= config.rotation {
            last_advance = Instant::now();
            index = (index + 1) % photos.len();
            needs_load = true;
        }

        if needs_load {
            needs_load = false;
            // Only ever one decoded photo is held; the previous texture is
            // dropped here, which is what keeps memory flat over a long run.
            current = load_current(
                &texture_creator,
                &mut photos,
                &mut index,
                &mut skip,
                screen_w,
                screen_h,
            );
            match &current {
                Some(t) => {
                    let q = t.query();
                    // Gated: this fires on every rotation, so at the default
                    // 30s that is ~2900 log lines a day, forever.
                    vlog!(
                        "Showing [{}/{}] {} ({}x{} texture)",
                        index + 1,
                        photos.len(),
                        photos[index].display(),
                        q.width,
                        q.height
                    );
                }
                None => vlog!("Nothing displayable; showing the waiting screen."),
            }
        }

        // Redraw and present EVERY tick, even when nothing changed.
        //
        // Presenting only on change is wrong under KMSDRM: `present` page-flips,
        // so a single draw leaves the image in a back buffer that is never
        // flipped in again and the panel keeps showing the previous (black)
        // front buffer. Re-presenting a static 800x480 software canvas is a few
        // MB/s of memcpy - free on a Pi 4, and worth far more as correctness.
        match &current {
            Some(texture) => {
                canvas.set_draw_color(Color::BLACK);
                canvas.clear();

                let query = texture.query();
                let (img_w, img_h) = (query.width as f32, query.height as f32);
                let scale = f32::min(screen_w as f32 / img_w, screen_h as f32 / img_h);
                let draw_w = (img_w * scale) as u32;
                let draw_h = (img_h * scale) as u32;
                let x = (screen_w.saturating_sub(draw_w) / 2) as i32;
                let y = (screen_h.saturating_sub(draw_h) / 2) as i32;

                canvas.copy(texture, None, Rect::new(x, y, draw_w, draw_h))?;
            }
            None => draw_waiting_screen(&mut canvas)?,
        }
        canvas.present();

        std::thread::sleep(TICK);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_image_extensions_case_insensitively() {
        for name in ["a.jpg", "a.JPG", "a.jpeg", "a.PNG", "a.WebP", "a.tiff"] {
            assert!(is_image(Path::new(name)), "{name} should be an image");
        }
    }

    #[test]
    fn rejects_non_images_and_partial_downloads() {
        // `.tmp` matters: a half-written download must never reach the decoder.
        for name in ["notes.txt", "photo.jpg.tmp", "manifest", "archive.zip"] {
            assert!(!is_image(Path::new(name)), "{name} should not be an image");
        }
    }

    #[test]
    fn lists_only_images_and_sorts_them() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.jpg", "a.png", "notes.txt", "c.jpg.tmp"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        // A subdirectory must not be offered to the decoder as a photo.
        std::fs::create_dir(dir.path().join("nested.jpg")).unwrap();

        let names = names_of(&list_photos(dir.path(), &SkipList::new()));
        assert_eq!(names, vec!["a.png", "b.jpg"]);
    }

    #[test]
    fn missing_directory_is_empty_not_fatal() {
        // First boot before the sync has created the folder: the frame must
        // show the waiting screen, not fall over.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(list_photos(&missing, &SkipList::new()).is_empty());
    }

    fn names_of(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn skipped_files_stay_out_of_the_listing() {
        // The rescan must not resurrect a file that failed to decode, or the
        // frame re-decodes and re-logs the same failure every 10 seconds.
        let dir = tempfile::tempdir().unwrap();
        for name in ["good.jpg", "corrupt.jpg"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }

        let bad = dir.path().join("corrupt.jpg");
        let mut skip = SkipList::new();
        skip.insert(bad.clone(), mtime(&bad));

        assert_eq!(names_of(&list_photos(dir.path(), &skip)), vec!["good.jpg"]);
    }

    #[test]
    fn a_replaced_file_gets_another_chance() {
        // Same name, new bytes (a re-upload): the recorded mtime no longer
        // matches, so the file returns to the rotation.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, b"broken").unwrap();

        let mut skip = SkipList::new();
        // A timestamp that cannot match the file's real one.
        skip.insert(path.clone(), Some(SystemTime::UNIX_EPOCH));

        assert_eq!(names_of(&list_photos(dir.path(), &skip)), vec!["photo.jpg"]);
    }
}
