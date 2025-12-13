use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

extern crate sdl2;

use sdl2::event::Event;
use sdl2::image::LoadTexture;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;

mod fetcher;

// #[tokio::main]
fn main() -> Result<(), Box<dyn Error>> {
    // Initiate values
    let account = std::env::var("AZURE_STORAGE_ACCOUNT").unwrap_or("benetmilian".to_string());
    let key = std::env::var("AZURE_STORAGE_KEY").unwrap_or("sp=rl&st=2025-12-08T20:55:51Z&se=2025-12-09T05:10:51Z&spr=https&sv=2024-11-04&sr=c&sig=GvfKQvawlrIRT47XjpMDP%2BHr2HMqUXkEJYy0rKp6Tgs%3D".to_string());
    let container = std::env::var("CONTAINER_NAME").unwrap_or("padrina".to_string());

    // Start by checking that we have a dir to sync photos
    let local_path = Path::new("./synced_photos");

    if let Err(e) = fs::create_dir(local_path) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            eprintln!("Failed to create directory {:?}: {}", local_path, e);
        }
    }

    // sync folder
    match fetcher::sync_folder(&account, &key, &container, &local_path) {
        Ok(_) => println!("Sync complete."),
        Err(e) => eprintln!("Error during sync: {}", e),
    }

    // init sdl 2
    let sdl_context = sdl2::init().unwrap();
    let video = sdl_context.video().unwrap();

    // Create a fullscreen window
    let window = video
        .window("Slideshow", 1920, 1080)
        .fullscreen()
        .build()
        .unwrap();

    let mut canvas = window.into_canvas().software().build().unwrap();

    let texture_creator = canvas.texture_creator();

    // load images from folder
    let folder = PathBuf::from("./synced_photos");
    let mut textures = vec![];

    for entry in fs::read_dir(folder)? {
        match entry {
            Ok(entry) => {
                if entry.file_name() != "manifest.txt" {
                    let path = entry.path();
                    let texture = texture_creator.load_texture(path.as_path())?;
                    textures.push(texture);
                }
            }
            Err(e) => eprintln!("Error reading directory entry: {}", e),
        }
    }

    if textures.is_empty() {
        eprintln!("No images found in the synced_photos directory.");
        return Ok(());
    }

    // Main loop
    let mut event_pump = sdl_context.event_pump().unwrap();
    let mut index = 0;

    loop {
        // Handle quit (ESC or window close)
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => return Ok(()),
                _ => {}
            }
        }

        // Clear screen
        canvas.set_draw_color(Color::BLACK);
        canvas.clear();

        // Get window size
        let (w, h) = canvas.output_size()?;

        // Query image size
        let query = textures[index].query();

        let img_w = query.width as f32;
        let img_h = query.height as f32;

        // Scale to fit fullscreen while keeping aspect ratio
        let scale = f32::min(w as f32 / img_w, h as f32 / img_h);

        let draw_w = (img_w * scale) as u32;
        let draw_h = (img_h * scale) as u32;

        let x = (w - draw_w) / 2;
        let y = (h - draw_h) / 2;

        let target = sdl2::rect::Rect::new(x as i32, y as i32, draw_w, draw_h);

        // Draw the image
        canvas.copy(&textures[index], None, target)?;

        canvas.present();

        // Wait 30 seconds
        thread::sleep(Duration::from_secs(30));

        // Next image
        index = (index + 1) % textures.len();
    }
}
