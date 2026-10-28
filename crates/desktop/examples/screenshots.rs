//! Render real game screens to PNG for the docs in `docs/images/`.
//!
//! Boots `Game` headless (no SDL window) once per scene, ticks it at the
//! normal 12 FPS so animations and behaviors have time to start, then dumps
//! the 128x64 framebuffer scaled up and tinted like the desktop simulator.
//!
//! Scenes other than the start scene are reached through the crash-resume
//! intent (`persist::intent_set`), which `Game::new` honors when a save
//! exists. A throwaway save is written first in a temp working directory,
//! so a real `catode32-save.json` is never touched.
//!
//! Usage (from the workspace root):
//!
//!     cargo run -p catode32-desktop --example screenshots \
//!         --target x86_64-unknown-linux-gnu -- docs/images

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use catode32_core::{
    context::GameContext,
    espnow_manager::EspNowManager,
    game::Game,
    input::{set_desktop_button, Button, ButtonPin, Buttons},
    led::Led,
    platform::{persist, radio::WIFI, rng::Rng},
    render::Renderer,
    save,
    scene::SceneId,
    storage,
};

const WIDTH: usize = 128;
const HEIGHT: usize = 64;
const SCALE: usize = 4;
const FRAME: Duration = Duration::from_millis(1000 / 12);

/// Same tint as the desktop simulator defaults.
const BG: [u8; 3] = [10, 10, 10];
const FG: [u8; 3] = [35, 165, 204];

/// One screenshot: which scene to boot into, how long to let it run, and
/// which buttons to tap (with frames to wait after each) before capturing.
struct Shot {
    file: &'static str,
    scene: Option<SceneId>,
    warmup: u32,
    taps: &'static [(Button, u32)],
}

const SHOTS: &[Shot] = &[
    Shot { file: "inside.png", scene: Some(SceneId::Inside), warmup: 36, taps: &[] },
    Shot { file: "outside.png", scene: Some(SceneId::Outside), warmup: 36, taps: &[] },
    Shot { file: "bedroom.png", scene: Some(SceneId::Bedroom), warmup: 36, taps: &[] },
    Shot { file: "kitchen.png", scene: Some(SceneId::Kitchen), warmup: 36, taps: &[] },
    Shot { file: "treehouse.png", scene: Some(SceneId::Treehouse), warmup: 36, taps: &[] },
    Shot { file: "main-menu.png", scene: Some(SceneId::Inside), warmup: 12, taps: &[(Button::Menu1, 6)] },
    Shot { file: "context-menu.png", scene: Some(SceneId::Inside), warmup: 12, taps: &[(Button::Menu2, 6)] },
    Shot { file: "stats.png", scene: Some(SceneId::Stats), warmup: 12, taps: &[] },
    Shot { file: "store.png", scene: Some(SceneId::Store), warmup: 12, taps: &[] },
    Shot { file: "forecast.png", scene: Some(SceneId::Forecast), warmup: 12, taps: &[] },
    Shot { file: "vacation-beach.png", scene: Some(SceneId::VacationBeach), warmup: 36, taps: &[] },
    Shot { file: "zoomies.png", scene: Some(SceneId::Zoomies), warmup: 12, taps: &[(Button::A, 30)] },
    Shot { file: "breakout.png", scene: Some(SceneId::Breakout), warmup: 12, taps: &[(Button::A, 24)] },
    Shot { file: "snake.png", scene: Some(SceneId::Snake), warmup: 12, taps: &[(Button::A, 24)] },
    Shot { file: "platformer.png", scene: Some(SceneId::Platformer), warmup: 12, taps: &[(Button::A, 24)] },
    Shot { file: "maze.png", scene: Some(SceneId::Maze), warmup: 12, taps: &[(Button::A, 12)] },
    Shot { file: "minigolf.png", scene: Some(SceneId::MiniGolf), warmup: 12, taps: &[(Button::A, 12)] },
];

fn main() {
    let out = PathBuf::from(env::args().nth(1).unwrap_or_else(|| "docs/images".into()));
    let out = env::current_dir().expect("cwd").join(out);
    fs::create_dir_all(&out).expect("create output dir");

    // Work inside a temp dir so the desktop save file lands there.
    let mut tmp = env::temp_dir();
    tmp.push(format!("catode32-screenshots-{}", std::process::id()));
    fs::create_dir_all(&tmp).expect("create tmp");
    env::set_current_dir(&tmp).expect("enter tmp");
    storage::init();

    // No save yet, so the first boot lands on the adoption screen.
    capture(&out, &Shot { file: "adoption.png", scene: None, warmup: 12, taps: &[] });

    // Write a save so later boots resume into the requested scene.
    let mut ctx = GameContext::new(Led::new());
    ctx.pet_name.clear();
    let _ = ctx.pet_name.push_str("Mochi");
    ctx.coins = 120;
    ctx.time_hours = 10;
    assert!(save::save(&mut ctx), "writing the seed save failed");

    for shot in SHOTS {
        capture(&out, shot);
    }

    let _ = fs::remove_dir_all(&tmp);
}

fn capture(out: &Path, shot: &Shot) {
    match shot.scene {
        Some(id) => persist::intent_set(id.to_u8()),
        None => persist::intent_clear(),
    }
    let mut game = new_game();
    run(&mut game, shot.warmup);
    for &(button, wait) in shot.taps {
        set_desktop_button(button as usize, true);
        run(&mut game, 2);
        set_desktop_button(button as usize, false);
        run(&mut game, wait);
    }

    let fb = game.renderer().framebuffer();
    let mut bits = [[false; WIDTH]; HEIGHT];
    for (y, row) in bits.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            *px = fb.pixel(x, y);
        }
    }
    let path = out.join(shot.file);
    fs::write(&path, encode_png(&bits)).expect("write png");
    println!("wrote {}", path.display());
}

fn new_game() -> Game {
    let buttons = Buttons::new([
        ButtonPin::new(0),
        ButtonPin::new(1),
        ButtonPin::new(2),
        ButtonPin::new(3),
        ButtonPin::new(4),
        ButtonPin::new(5),
        ButtonPin::new(6),
        ButtonPin::new(7),
    ]);
    Game::new(
        Renderer::new(),
        buttons,
        Rng::new(),
        Led::new(),
        WIFI::new(),
        EspNowManager::new(),
    )
}

fn run(game: &mut Game, frames: u32) {
    for _ in 0..frames {
        game.tick();
        thread::sleep(FRAME);
    }
}

/// Minimal 1-bit palette PNG writer (stored deflate blocks, no compression
/// library needed). Scales each game pixel to SCALE x SCALE.
fn encode_png(bits: &[[bool; WIDTH]; HEIGHT]) -> Vec<u8> {
    let w = WIDTH * SCALE;
    let h = HEIGHT * SCALE;
    let row_bytes = w.div_ceil(8);

    let mut raw = Vec::with_capacity(h * (row_bytes + 1));
    for y in 0..h {
        raw.push(0); // filter: none
        let src = &bits[y / SCALE];
        for b in 0..row_bytes {
            let mut byte = 0u8;
            for i in 0..8 {
                let x = b * 8 + i;
                if x < w && src[x / SCALE] {
                    byte |= 0x80 >> i;
                }
            }
            raw.push(byte);
        }
    }

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[1, 3, 0, 0, 0]); // 1-bit, palette
    chunk(&mut png, b"IHDR", &ihdr);
    let mut plte = Vec::new();
    plte.extend_from_slice(&BG);
    plte.extend_from_slice(&FG);
    chunk(&mut png, b"PLTE", &plte);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        out.push(if blocks.peek().is_none() { 1 } else { 0 });
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
