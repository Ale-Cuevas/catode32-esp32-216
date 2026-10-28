//! Desktop simulator for catode32.
//!
//! Drives `catode32-core` (built with the `desktop` feature) inside an
//! SDL2 window via `embedded-graphics-simulator`. Each game frame:
//!
//! 1. Pump SDL events; map arrow / A / S / Q / W keys onto the
//!    `Button` indices used by `catode32_core::input`.
//! 2. Call `Game::tick()` to run one frame of game logic + drawing.
//! 3. Blit the in-memory 128x64 framebuffer to the `SimulatorDisplay`.
//! 4. `Window::update()` and sleep enough to land near 12 FPS.
//!
//! ESC or closing the window saves the game, then quits.
//!
//! The lit-pixel and background colors can be overridden at launch with
//! `--fg RRGGBB` / `--bg RRGGBB` (see `USAGE`).
//!
//! `--amoled` previews the ESP32-S3-Touch-AMOLED-2.16 port instead: a
//! 480x480 window composed by `catode32-amoled216` (the same layout, scaler
//! and touch hit-test the firmware uses), with the mouse acting as a finger
//! on the on-screen controls. The keyboard still works alongside it.

use std::process;
use std::thread;
use std::time::{Duration, Instant};

use catode32_amoled216 as amoled;
use catode32_core::{
    espnow_manager::EspNowManager,
    game::Game,
    input::{set_desktop_button, ButtonPin, Buttons},
    led::Led,
    platform::{radio::WIFI, rng::Rng},
    render::Renderer,
    storage,
};

use embedded_graphics::{
    pixelcolor::Rgb888,
    prelude::{DrawTarget, Point, RgbColor, Size},
    Pixel,
};
use embedded_graphics_simulator::{
    sdl2::{Keycode, MouseButton},
    OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 64;
const SCALE: u32 = 8;
const FPS: u64 = 12;
const FRAME_MS: u64 = 1000 / FPS;

/// Default "lit pixel" tint.
const DEFAULT_FG: Rgb888 = Rgb888::new(35, 165, 204);
/// Default background (a near-black charcoal, not pure 0,0,0 so the off
/// pixels read as "screen, not void"). AMOLED mode defaults to true black
/// instead, like the real panel.
const DEFAULT_BG: Rgb888 = Rgb888::new(10, 10, 10);

const USAGE: &str = "\
Usage: catode32-desktop [--fg RRGGBB] [--bg RRGGBB] [--amoled]

Options:
  --fg RRGGBB   Lit pixel color (default 23A5CC)
  --bg RRGGBB   Background color (default 0A0A0A, or 000000 with --amoled)
  --amoled      Preview the 480x480 AMOLED port: click the on-screen controls
  -h, --help    Print this help

Colors are 6-digit hex, with or without a leading '#'.";

/// Command-line options. Colors are `None` unless overridden, since the
/// default background depends on the mode.
struct Options {
    fg: Option<Rgb888>,
    bg: Option<Rgb888>,
    amoled: bool,
}

/// Colors used to blit the 1-bit framebuffer to the window.
struct Palette {
    fg: Rgb888,
    bg: Rgb888,
}

/// Parse `RRGGBB` or `#RRGGBB` (any case) into a color.
fn parse_hex(s: &str) -> Option<Rgb888> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Rgb888::new(channel(0)?, channel(2)?, channel(4)?))
}

/// Print an error plus usage to stderr and exit with status 2.
fn usage_error(msg: &str) -> ! {
    eprintln!("error: {msg}\n\n{USAGE}");
    process::exit(2);
}

/// Read `--fg` / `--bg` overrides and `--amoled` from the command line.
fn parse_args() -> Options {
    let mut opts = Options {
        fg: None,
        bg: None,
        amoled: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            println!("{USAGE}");
            process::exit(0);
        }
        if arg == "--amoled" {
            opts.amoled = true;
            continue;
        }
        let (flag, inline_value) = match arg.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (arg, None),
        };
        let slot = match flag.as_str() {
            "--fg" => &mut opts.fg,
            "--bg" => &mut opts.bg,
            _ => usage_error(&format!("unknown argument '{flag}'")),
        };
        let value = inline_value
            .or_else(|| args.next())
            .unwrap_or_else(|| usage_error(&format!("{flag} needs a color value")));
        *slot = Some(
            parse_hex(&value)
                .unwrap_or_else(|| usage_error(&format!("invalid color '{value}' for {flag}"))),
        );
    }
    opts
}

/// Map an SDL keycode onto the firmware-side button index.
/// Arrows for the d-pad, A=A, S=B, Q=MENU1, W=MENU2.
fn button_index(key: Keycode) -> Option<usize> {
    match key {
        Keycode::Up => Some(0),
        Keycode::Down => Some(1),
        Keycode::Left => Some(2),
        Keycode::Right => Some(3),
        Keycode::A => Some(4),
        Keycode::S => Some(5),
        Keycode::Q => Some(6),
        Keycode::W => Some(7),
        _ => None,
    }
}

fn main() {
    // Before anything else, so `--help` / bad flags exit without touching
    // the save file.
    let opts = parse_args();

    storage::init();

    let renderer = Renderer::new();
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
    let rng = Rng::new();
    let led = Led::new();
    let wifi = WIFI::new();
    let espnow = EspNowManager::new();

    let mut game = Game::new(renderer, buttons, rng, led, wifi, espnow);

    let mut view = if opts.amoled {
        View::amoled(&opts)
    } else {
        View::classic(&opts)
    };
    let mut window = Window::new(view.title(), &view.output_settings());
    // Prime the window so `events()` has a valid SDL window to pump from
    // on the first iteration.
    window.update(view.display());

    // Buttons that received both KeyDown and KeyUp in a single pump are
    // held pressed across one tick and released afterwards, so brief taps
    // that land entirely within a frame still register. Clicks on the
    // AMOLED controls get the same treatment through `touch_tap`.
    let mut key_held = [false; 8];
    // Mouse position while the left button is down (AMOLED mode only).
    let mut touch: Option<Point> = None;

    'outer: loop {
        let frame_start = Instant::now();

        let mut key_tap = [false; 8];
        let mut touch_tap = 0u8;

        for ev in window.events() {
            match ev {
                SimulatorEvent::Quit => break 'outer,
                SimulatorEvent::KeyDown { keycode, .. } => {
                    if keycode == Keycode::Escape {
                        break 'outer;
                    }
                    if let Some(idx) = button_index(keycode) {
                        key_held[idx] = true;
                        key_tap[idx] = true;
                    }
                }
                SimulatorEvent::KeyUp { keycode, .. } => {
                    if let Some(idx) = button_index(keycode) {
                        key_held[idx] = false;
                    }
                }
                SimulatorEvent::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    point,
                } if opts.amoled => {
                    touch = Some(point);
                    touch_tap |= touch_mask(point);
                }
                SimulatorEvent::MouseMove { point } if touch.is_some() => touch = Some(point),
                SimulatorEvent::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    ..
                } => touch = None,
                _ => {}
            }
        }

        let touch_held = touch.map_or(0, touch_mask);
        let mut mask = 0u8;
        for idx in 0..8 {
            let touched = (touch_held | touch_tap) & (1 << idx) != 0;
            let pressed = key_held[idx] || key_tap[idx] || touched;
            set_desktop_button(idx, pressed);
            if pressed {
                mask |= 1 << idx;
            }
        }

        game.tick();

        // Release taps that ended within the frame; held keys/touches stay.
        for idx in 0..8 {
            if !key_held[idx] && touch_held & (1 << idx) == 0 {
                set_desktop_button(idx, false);
            }
        }

        view.draw(game.renderer(), mask);
        window.update(view.display());

        let elapsed = frame_start.elapsed();
        let target = Duration::from_millis(FRAME_MS);
        if elapsed < target {
            thread::sleep(target - elapsed);
        }
    }

    // ESC or closing the window: save before exiting.
    game.save();
}

/// Button mask for a click at `point` on the 480x480 AMOLED window.
fn touch_mask(point: Point) -> u8 {
    let (Ok(x), Ok(y)) = (u16::try_from(point.x), u16::try_from(point.y)) else {
        return 0;
    };
    amoled::touch::hit_point(x, y)
}

/// What the window shows: the classic scaled-up 128x64 OLED, or the
/// 480x480 AMOLED port preview.
enum View {
    Classic {
        display: SimulatorDisplay<Rgb888>,
        palette: Palette,
    },
    Amoled {
        display: SimulatorDisplay<Rgb888>,
        palette: amoled::scale::Palette,
        screen: Vec<u8>,
    },
}

impl View {
    fn classic(opts: &Options) -> Self {
        View::Classic {
            display: SimulatorDisplay::new(Size::new(WIDTH, HEIGHT)),
            palette: Palette {
                fg: opts.fg.unwrap_or(DEFAULT_FG),
                bg: opts.bg.unwrap_or(DEFAULT_BG),
            },
        }
    }

    fn amoled(opts: &Options) -> Self {
        let to565 = |c: Rgb888| amoled::scale::rgb888_to_565(c.r(), c.g(), c.b());
        let default = amoled::scale::Palette::DEFAULT;
        let side = amoled::layout::SCREEN_W as u32;
        View::Amoled {
            display: SimulatorDisplay::new(Size::new(side, side)),
            palette: amoled::scale::Palette {
                fg: opts.fg.map_or(default.fg, to565),
                bg: opts.bg.map_or(default.bg, to565),
            },
            screen: vec![0; amoled::SCREEN_BYTES],
        }
    }

    fn title(&self) -> &'static str {
        match self {
            View::Classic { .. } => "catode32",
            View::Amoled { .. } => "catode32 (AMOLED 480x480)",
        }
    }

    fn output_settings(&self) -> embedded_graphics_simulator::OutputSettings {
        let scale = match self {
            View::Classic { .. } => SCALE,
            View::Amoled { .. } => 1,
        };
        OutputSettingsBuilder::new().scale(scale).build()
    }

    fn display(&self) -> &SimulatorDisplay<Rgb888> {
        match self {
            View::Classic { display, .. } | View::Amoled { display, .. } => display,
        }
    }

    fn draw(&mut self, renderer: &Renderer, pressed: u8) {
        match self {
            View::Classic { display, palette } => blit(renderer, display, palette),
            View::Amoled {
                display,
                palette,
                screen,
            } => blit_amoled(renderer, display, *palette, screen, pressed),
        }
    }
}

fn blit(renderer: &Renderer, target: &mut SimulatorDisplay<Rgb888>, palette: &Palette) {
    let fb = renderer.framebuffer();
    let invert = fb.invert();
    let fg = palette.fg;

    let _ = target.clear(palette.bg);
    let pixels = (0..HEIGHT as i32).flat_map(|y| {
        (0..WIDTH as i32).filter_map(move |x| {
            let lit = fb.pixel(x as usize, y as usize) ^ invert;
            if lit {
                Some(Pixel(Point::new(x, y), fg))
            } else {
                None
            }
        })
    });
    let _ = target.draw_iter(pixels);
}

/// Compose the full AMOLED screen with the firmware's code path, then
/// convert its RGB565 back to the simulator's RGB888.
fn blit_amoled(
    renderer: &Renderer,
    target: &mut SimulatorDisplay<Rgb888>,
    palette: amoled::scale::Palette,
    screen: &mut [u8],
    pressed: u8,
) {
    let fb = renderer.framebuffer();
    // Display off (soft sleep): the panel goes fully black, controls too.
    if !fb.is_on() {
        let _ = target.clear(rgb565_to_888(palette.bg));
        return;
    }

    let mut src = [0u8; amoled::scale::SRC_BYTES];
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            if fb.pixel(x, y) {
                src[y * WIDTH as usize / 8 + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    amoled::compose_screen(&src, pressed, palette, fb.invert(), screen);

    let side = amoled::layout::SCREEN_W as usize;
    let pixels = screen.as_chunks::<2>().0.iter().enumerate().map(|(i, &px)| {
        let c = u16::from_be_bytes(px);
        Pixel(Point::new((i % side) as i32, (i / side) as i32), rgb565_to_888(c))
    });
    let _ = target.draw_iter(pixels);
}

fn rgb565_to_888(c: u16) -> Rgb888 {
    let r = ((c >> 11) & 0x1F) as u8;
    let g = ((c >> 5) & 0x3F) as u8;
    let b = (c & 0x1F) as u8;
    Rgb888::new((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_valid_colors() {
        assert_eq!(parse_hex("23a5cc"), Some(Rgb888::new(35, 165, 204)));
        assert_eq!(parse_hex("#23A5CC"), Some(Rgb888::new(35, 165, 204)));
        assert_eq!(parse_hex("0a0A0a"), Some(Rgb888::new(10, 10, 10)));
        assert_eq!(parse_hex("#ffffff"), Some(Rgb888::new(255, 255, 255)));
    }

    #[test]
    fn parse_hex_rejects_invalid_colors() {
        assert_eq!(parse_hex(""), None);
        assert_eq!(parse_hex("#"), None);
        assert_eq!(parse_hex("fff"), None);
        assert_eq!(parse_hex("1234567"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
        assert_eq!(parse_hex("##123456"), None);
        assert_eq!(parse_hex("+12345"), None);
    }

    #[test]
    fn rgb565_round_trip_extremes() {
        assert_eq!(rgb565_to_888(0x0000), Rgb888::new(0, 0, 0));
        assert_eq!(rgb565_to_888(0xFFFF), Rgb888::new(255, 255, 255));
        assert_eq!(rgb565_to_888(0xF800), Rgb888::new(255, 0, 0));
    }

    #[test]
    fn clicks_outside_window_hit_nothing() {
        assert_eq!(touch_mask(Point::new(-5, 300)), 0);
        assert_eq!(touch_mask(Point::new(240, 100)), 0);
    }
}
