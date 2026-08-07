//! MarcDigital frame: sync photos from Azure, then run a fullscreen slideshow.
//!
//! The frame is unattended — nobody can reach a shell to restart it — so the
//! slideshow is written to survive the things that actually happen in the
//! field: a corrupt JPEG, an empty photo folder on first boot, a folder whose
//! contents change underneath it, and a display that must stay responsive to a
//! quit request.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

/// Candidate image files, sorted so rotation order is stable across restarts
/// and re-scans. A folder we cannot read is logged and treated as empty — that
/// is a degraded state to display, not a reason to exit.
fn list_photos(dir: &Path) -> Vec<PathBuf> {
    let mut photos: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && is_image(path))
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
/// A corrupt or truncated image is dropped from `photos` and logged, so the
/// frame moves on instead of dying. Returns `None` only when nothing in the
/// list could be decoded at all.
fn load_current<'a>(
    creator: &'a TextureCreator<WindowContext>,
    photos: &mut Vec<PathBuf>,
    index: &mut usize,
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
                // is removed from the rotation for this run.
                eprintln!("Skipping {}: {e}", photos[*index].display());
                photos.remove(*index);
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::from_env()?;

    let photo_store = AzureBlobStore::new(
        &config.tenant_id,
        &config.client_id,
        &config.client_secret,
        &config.storage_account,
        &config.container,
    )?;

    // A failed sync is survivable: we still have whatever is already on disk.
    match run_sync(&photo_store, &config.photo_dir).await {
        Ok(plan) => println!(
            "Sync complete: {} downloaded, {} deleted.",
            plan.to_download.len(),
            plan.to_delete.len()
        ),
        Err(e) => eprintln!("Error during sync: {:#}", e),
    }

    let sdl_context = sdl2::init()?;
    let video = sdl_context.video()?;

    let window = video.window("Slideshow", 1920, 1080).fullscreen().build()?;

    let mut canvas = window.into_canvas().software().build()?;
    let texture_creator = canvas.texture_creator();

    // Decode target: the panel's own resolution, so textures are never larger
    // than what can actually be shown.
    let (mut screen_w, mut screen_h) = canvas.output_size()?;

    let mut photos = list_photos(&config.photo_dir);
    if photos.is_empty() {
        // Not an error, and explicitly not an exit: the first sync may not have
        // landed yet, and under `restart: always` exiting here is a crash loop.
        println!(
            "No photos in {} yet — showing the waiting screen.",
            config.photo_dir.display()
        );
    }

    let mut index = 0usize;
    let mut current: Option<Texture> = None;
    let mut needs_load = true;
    let mut needs_redraw = true;
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
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        // Pick up photos a sync has added or removed, without a restart.
        if last_rescan.elapsed() >= RESCAN_INTERVAL {
            last_rescan = Instant::now();
            let found = list_photos(&config.photo_dir);
            if found != photos {
                // Stay on the photo currently displayed if it survived, so a
                // sync does not visibly jolt the rotation.
                let showing = photos.get(index).cloned();
                photos = found;
                index = showing
                    .and_then(|path| photos.iter().position(|p| *p == path))
                    .unwrap_or(0);
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
                screen_w,
                screen_h,
            );
            needs_redraw = true;
        }

        if needs_redraw {
            needs_redraw = false;
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
        }

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

        let names: Vec<String> = list_photos(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, vec!["a.png", "b.jpg"]);
    }

    #[test]
    fn missing_directory_is_empty_not_fatal() {
        // First boot before the sync has created the folder: the frame must
        // show the waiting screen, not fall over.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(list_photos(&missing).is_empty());
    }
}
