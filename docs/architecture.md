# Code architecture

How the Rust workspace is put together: which crate does what, how the same game code runs on the ESP32 and on a desktop, and where hardware-specific code lives. For what the game *does* (behaviors, stats, scenes), see [game-logic.md](game-logic.md).

## Crates

```mermaid
flowchart TB
    subgraph workspace["Cargo workspace"]
        firmware["<b>catode32-firmware</b><br/>crates/firmware<br/>no_std binary for ESP32-C6 / C3"]
        desktop["<b>catode32-desktop</b><br/>crates/desktop<br/>SDL2 simulator binary"]
        core["<b>catode32-core</b><br/>crates/core<br/>all game logic, scenes, assets"]
        i18n["<b>catode32-i18n-macros</b><br/>crates/i18n-macros<br/>t!() proc macro + translations/*.json"]
    end

    firmware -- "features: c6 or c3, lang-*" --> core
    desktop -- "features: desktop, lang-*" --> core
    core --> i18n

    core -. "c6 / c3 only" .-> esp["esp-hal, esp-radio, esp-rtos,<br/>esp-storage, esp-println, ssd1306"]
    desktop --> sim["embedded-graphics-simulator<br/>(SDL2)"]
    core --> eg["embedded-graphics, heapless,<br/>serde-json-core, micromath"]
```

| Crate | Role |
| --- | --- |
| `core` | Everything that makes the game: game loop, scenes, pet behaviors, world systems (time, weather, plants), UI widgets, save format, sprites. `no_std` on firmware, `std` when built with `desktop`. |
| `firmware` | About 60 lines. Sets up the heap (64 KB, used by the Wi-Fi stack), the watchdog, `board::init`, storage, the `esp_rtos` scheduler, then calls `Game::new(...).run()`. `panic.rs` saves the panic message and backtrace to RTC RAM and resets. |
| `desktop` | Opens an SDL window, maps keys to the 8 buttons, calls `Game::tick()` once per frame and draws the framebuffer scaled 8x. With `--amoled` it shows the 480x480 ESP32-S3-Touch-AMOLED-2.16 layout instead, using `crates/amoled216`, and the mouse acts as touch. Also has `examples/screenshots.rs`, which generated the images in `docs/images/` (including `amoled.png`). |
| `amoled216` | Work in progress for the ESP32-S3-Touch-AMOLED-2.16 port. `no_std`, with no hardware code: the 480x480 layout (3x game viewport plus touch controls), the 1bpp→RGB565 strip scaler, the control rasterizer and touch→button-mask hit-testing. The desktop `--amoled` mode uses it now, and the S3 firmware will use it later. `crates/s3-hello` is a separate S3 toolchain smoke test outside this workspace. |
| `i18n-macros` | `t!("key")` becomes a string literal at compile time, picked from the JSON for the active `lang-*` feature, falling back to English and then to the key. No runtime cost and no string tables in RAM. |

## Feature flags select the backend

There's no runtime "platform" switch. The backend is chosen at compile time by Cargo features on `catode32-core`:

```mermaid
flowchart LR
    subgraph features["catode32-core features"]
        c6["c6"]
        c3["c3"]
        dk["desktop"]
        lang["lang-en / de / es / fr / it / nl<br/>(exactly one)"]
    end
    c6 --> fw["Firmware backend<br/>esp-hal GPIO, I2C SSD1306,<br/>RMT LED, flash, Wi-Fi, ESP-NOW"]
    c3 --> fw
    dk --> host["Desktop backend<br/>atomic virtual pins, in-memory<br/>framebuffer, JSON save file,<br/>no-op radio and LED"]
    lang --> i18n["t!() resolves strings<br/>from that language's JSON"]
```

`board.rs` raises a `compile_error!` unless exactly one of `c6` / `c3` is enabled for a firmware build. The default build target in `.cargo/config.toml` is `riscv32imac-unknown-none-elf`, which is why host builds need `--target x86_64-unknown-linux-gnu`.

## The platform seam

The central rule of `core`: game code never imports `esp_hal` or `esp_println` directly. Anything that touches hardware lives behind a small set of modules, each with a firmware version and a desktop version selected by `#[cfg(feature = "desktop")]`.

```mermaid
flowchart TB
    subgraph game["Hardware-agnostic game code"]
        g["game.rs<br/>Game loop"]
        sc["scenes/*<br/>locations, minigames, menus"]
        bh["behavior.rs + behaviors/*<br/>pet AI"]
        sys["time_system, weather_system,<br/>plant_system, sky, save.rs"]
        ui["ui/*<br/>menus, popups, keyboard"]
    end

    subgraph seam["Hardware seam (cfg-switched twins)"]
        render["render.rs<br/>Renderer"]
        input["input.rs<br/>Buttons / ButtonPin"]
        led["led.rs<br/>Led"]
        storage["storage.rs<br/>flash ring vs JSON file"]
        board["board.rs<br/>pin map, init (firmware only)"]
        platform["platform/*<br/>time, rng, persist, power,<br/>radio, watchdog"]
        radio["radio.rs, wifi_tracker.rs,<br/>espnow_manager.rs"]
    end

    game --> seam

    subgraph hw["ESP32-C6 / C3"]
        oled["SSD1306 128x64 OLED (I2C)"]
        btn["8 GPIO buttons"]
        ws["WS2812 LED (RMT)"]
        flash["nvs flash partition"]
        rtc["RTC fast RAM"]
        wifi["Wi-Fi / ESP-NOW"]
    end

    subgraph host["Desktop"]
        fb["DesktopFramebuffer (1 KB, 1bpp)"]
        keys["AtomicBool virtual pins"]
        file["./catode32-save.json"]
        stub["no-op stubs"]
    end

    render --> oled
    render --> fb
    input --> btn
    input --> keys
    led --> ws
    storage --> flash
    storage --> file
    platform --> rtc
    radio --> wifi
    radio --> stub
```

What each seam module does:

| Module | Firmware | Desktop |
| --- | --- | --- |
| `render.rs` `Renderer` | Buffered `Ssd1306` over I2C | `DesktopFramebuffer`, a 1024-byte 1bpp buffer the desktop binary reads after each tick |
| `input.rs` `ButtonPin` | `esp_hal::gpio::Input` with pull-up | `AtomicBool` "virtual pin" set by `set_desktop_button` |
| `led.rs` `Led` | WS2812 via RMT on GPIO8 | No-op |
| `storage.rs` | Ring of 4 KB sectors in the 24 KB `nvs` partition, payload written before header for crash safety | `./catode32-save.json` in the working directory |
| `platform/time.rs` | esp-hal monotonic clock | `std::time::Instant` |
| `platform/persist.rs` | RTC fast RAM: scene-change intent and last panic, survive resets | Plain statics, lost on exit |
| `platform/radio.rs`, `radio.rs`, `espnow_manager.rs` | `esp-radio` Wi-Fi scans and ESP-NOW, radio acquired only while needed | Never-constructed stubs; scenes see an empty inbox |
| `platform/power.rs`, `watchdog.rs` | Software reset, task watchdog | No-ops |
| `game.rs` `enter_deep_sleep` | C6 LP-IO wake on GPIO0-3, device resets on wake | Exits the process |

### Where the S3 AMOLED port plugs in

The port (see `PORT_PLAN_S3_AMOLED.md`) adds a third backend at the same seam instead of touching game code. The plan keeps the game at 128x64 1bpp and scales it at flush time:

```mermaid
flowchart LR
    game["Game draws into<br/>128x64 1bpp buffer"] --> flush["Renderer::flush()<br/>(new s3 backend)"]
    flush --> scale["Scale 3x to 384x192,<br/>tint fg/bg to RGB565,<br/>stream in strips"]
    scale --> amoled["CO5300 480x480<br/>AMOLED over QSPI"]
    touch["CST9220 touch<br/>(I2C)"] --> hit["Hit-test on-screen<br/>D-pad / A / B / Menu"]
    hit --> mask["8-bit button mask"]
    mask --> pins["s3 ButtonPin::is_low()"]
    pins --> game
```

Files the port touches: `core/Cargo.toml` and `firmware/Cargo.toml` (an `s3` feature), `board.rs`, `render.rs`, `input.rs`, `led.rs`, `game.rs` (input poll hook, deep sleep), `firmware/src/main.rs`, and `.cargo/config.toml` (Xtensa target). Use the `waveshare-amoled-216` skill for pins and drivers.

## Boot sequence

```mermaid
sequenceDiagram
    participant FW as firmware main()
    participant B as board::init
    participant S as storage
    participant G as Game::new
    participant P as persist (RTC RAM)
    participant SM as SceneManager

    FW->>FW: heap 64 KB, CPU max clock, esp_hal::init
    FW->>FW: watchdog::start
    FW->>B: peripherals
    B-->>FW: Board (i2c, buttons, rng, led, flash, wifi)
    FW->>S: storage::init(flash)
    FW->>FW: Renderer::new(i2c), esp_rtos::start
    FW->>G: Game::new(renderer, buttons, rng, led, wifi, espnow)
    G->>P: reset_reason(), crash_take()
    G->>S: save::load(context)
    G->>P: intent_peek()
    alt save loaded and resumable intent (< 2 attempts)
        G->>SM: start in the interrupted scene
    else save loaded
        G->>SM: start in Inside (living room)
    else no save
        G->>SM: start in Adoption
    end
    FW->>G: run() loop at 12 FPS, feeding the watchdog
```

The "intent" is a small crash-recovery mechanism. Before each scene swap the target scene ID is written to RTC RAM (no flash wear). If the device resets mid-swap, the next boot resumes into that scene, but gives up after two attempts so a scene that crashes on entry can't cause a boot loop.

## Game loop (one frame)

`Game::tick()` is the whole frame. The firmware's `run()` calls it and then busy-waits to keep 12 FPS. The desktop binary calls it from its own loop.

```mermaid
flowchart TD
    start(["tick()"]) --> dt["dt = time since last frame"]
    dt --> act{"any button<br/>pressed?"}
    act -- yes --> notify["sleep_manager.notify_activity()"]
    act -- no --> poll
    notify --> poll["espnow.poll(): pull inbound frames into inbox"]
    poll --> upd["update(dt * time_speed)"]

    subgraph update["update()"]
        ts["time_system.advance<br/>clock, day, season, moon,<br/>temperature, weather"] --> su["scene_manager.update<br/>(active scene or menu overlay)"]
        su --> req{"scene swap<br/>requested?"}
        req -- yes --> tr["store pending_scene,<br/>start transition"]
        req -- no --> tu
        tr --> tu["transition.update"]
        tu --> mid{"Midpoint?<br/>(screen fully black)"}
        mid -- yes --> swap["swap scene, maybe Wi-Fi scan + save,<br/>or enter sleep / deep sleep"]
    end

    upd --> update
    update --> draw["draw(): clear, scene.draw, transition.draw, flush"]
    draw --> power["handle pending power action<br/>(reboot / light sleep / deep sleep)"]
    power --> idle{"idle for 15 min?"}
    idle -- yes --> sleep["start transition, then sleep_pending"]
    idle -- no --> done(["end of frame"])
    sleep --> done
```

Things to know about the loop:

- `dt` is scaled by `context.time_speed` exactly once, in `Game::update`. Subsystems must not scale it again.
- Scene swaps never happen immediately. A scene returns `Some(SceneId)` from `update`, the screen fades out, and the swap happens at the transition midpoint while the screen is black. Slow work (Wi-Fi scans take 1-3 s) is hidden there too.
- Opening the main menu (`SceneId::Menu`) is special: `SceneManager` shows it as an overlay without exiting the current scene, and keeps the scene alive via `tick_background` so the pet keeps behaving while you browse.

## Scene system

```mermaid
classDiagram
    class Scene {
        <<trait>>
        +enter(ctx)
        +exit(ctx)
        +update(ctx, buttons, dt) Option~SceneId~
        +draw(ctx, renderer, dt_ms)
        +tick_background(ctx, dt)
        +mark_behavior_almost_done(ctx)
    }
    class SceneManager {
        -current: ActiveScene
        -current_id: SceneId
        -overlay: Option~MenuScene~
        +update() Option~SceneId~
        +swap_to(ctx, next)
        +sleep_update()
        +draw()
    }
    class ActiveScene {
        <<enum>>
        Inside(InsideScene)
        Outside(OutsideScene)
        Zoomies(ZoomiesScene)
        +50 variants total
        +from_id(SceneId)
        +as_scene() dyn Scene
    }
    class LocationScene {
        environment / camera
        character
        BehaviorManager
        sky, plants, Menu2 context menu
        ESP-NOW visit logic
    }
    class InsideScene
    class OutsideScene
    class VacationBase
    class ZoomiesScene

    SceneManager --> ActiveScene
    SceneManager --> MenuScene : overlay
    ActiveScene ..|> Scene
    InsideScene ..|> Scene
    OutsideScene ..|> Scene
    ZoomiesScene ..|> Scene
    InsideScene *-- LocationScene
    OutsideScene *-- LocationScene
    VacationBase *-- LocationScene
```

- `SceneId` (in `scene.rs`) is a `#[repr(u8)]` enum. `SceneId::ALL` must stay in declaration order (a const assert checks it), because the numeric ID is stored for crash-resume.
- `ActiveScene` is generated by the `dispatch_enum!` macro in `scenes/mod.rs`. It is an enum holding exactly one live scene, so there's no heap allocation and only the active scene's state is in RAM.
- The five home locations and the four vacation spots are thin wrappers around `LocationScene`, which handles the camera, the pet character, behaviors, sky and weather, plants, the Menu2 context menu and playdate visits.
- Minigames, menus and debug screens implement `Scene` directly.

## Pet AI structure

```mermaid
classDiagram
    class BehaviorManager {
        -current: ActiveBehavior
        +update(ctx, character, dt)
        +trigger(next)
        -advance(completed)
        -accumulate_sickness()
    }
    class Behavior {
        <<trait>>
        +id() BehaviorId
        +progress() f32
        +pose() PoseId
        +enter / update / exit
        +apply_completion_bonus(ctx, progress)
        +next(ctx) Option~NextBehavior~
        +draw()
    }
    class ActiveBehavior {
        <<enum>>
        Idle, Sleeping, Napping, Hunting
        Playing, Eating, Zoomies (29 total)
    }
    class auto_select {
        <<function>>
        priority per behavior
        recency penalty
        bin and random pick
    }
    class GameContext {
        18 stats + sickness
        inventory, coins, plants
        time, weather, wifi lists
        recent_behaviors
    }

    LocationScene --> BehaviorManager
    BehaviorManager --> ActiveBehavior
    ActiveBehavior ..|> Behavior
    BehaviorManager ..> auto_select : when chain ends
    Behavior ..> GameContext : reads stats, applies bonuses
    auto_select ..> GameContext
```

See [game-logic.md](game-logic.md#how-the-pet-decides-what-to-do) for how behaviors are picked.

## State and persistence

```mermaid
flowchart LR
    ctx["GameContext<br/>(single struct, all game state)"]
    ctx -- "save::save()<br/>serde-json-core into fixed buffer" --> json["JSON payload<br/>(≤ storage::MAX_PAYLOAD)"]
    json -- firmware --> ring["nvs partition ring buffer<br/>header: magic, seq, len"]
    json -- desktop --> file["catode32-save.json"]
    ring -- "save::load() on boot<br/>(highest valid sequence)" --> ctx
    file --> ctx
```

When saves happen:

- Before each hourly Wi-Fi scan, at a transition midpoint. This doubles as the regular save.
- `save_if_needed`, called while the pet is sleeping or napping, saves if more than 75 minutes have passed since the last save.
- Before a user-requested reboot, and when the desktop simulator quits. Never during adoption.

`GameContext` uses `heapless` collections with fixed caps (see the `*_MAX` constants in `context.rs`) so RAM use is fixed. Save keys use stable English identifiers, never translated labels, so saves work across language builds. A save-shape change must stay backward compatible or bump `SCHEMA_VERSION` in `save.rs`.

## Radio usage

```mermaid
sequenceDiagram
    participant Game
    participant Radio as radio::acquire / release
    participant WT as wifi_tracker
    participant EN as EspNowManager
    participant Peer as Other Catode32

    Note over Game: Hourly, at a transition midpoint
    Game->>Game: save (scan_started = true)
    Game->>WT: scan_now
    WT->>Radio: acquire
    WT->>WT: scan APs, update familiar / recent lists
    WT->>Radio: release
    WT-->>Game: in_familiar_location (pet feels "at home")

    Note over Game,Peer: Social scene / outdoor locations
    Game->>Radio: acquire (radio_users += 1)
    Game->>EN: start
    EN-)Peer: broadcast invite / vocalize
    Peer-)EN: vst state (x, pose, behavior) every 3 s
    EN-->>Game: inbox drained by LocationScene
    Game->>Radio: release on scene exit
```

The radio is never on at rest. Code acquires it, uses it, and releases it; a reference count (`radio_users`) handles overlapping users.

## Code conventions worth knowing

- Use the crate's `println!` (from `lib.rs`), not `esp_println::println!`. It works on both backends.
- Use the macros in `macros.rs` (`enum_str_method!`, `enum_key_pair_option!`, `enum_key_pair_default!`, `dispatch_enum!`) for enum-to-string tables and enum dispatch.
- Sprites and art live as Rust consts in `core/src/assets/`. The art is CC BY-NC-ND (see `LICENSE`), so don't modify it.
- Most code avoids allocation. Firmware has a 64 KB heap that exists only for the Wi-Fi stack.
