# Game logic

How Catode32 plays: what the player sees, how the pet decides what to do, how stats, time, weather, sickness and coins connect. For how the code is organized, see [architecture.md](architecture.md).

All screenshots below are real frames rendered by the game (128x64, shown 4x larger with the desktop simulator's colors). Regenerate them with:

```bash
cargo run -p catode32-desktop --example screenshots --target x86_64-unknown-linux-gnu -- docs/images
```

## The big picture

You never control the cat directly. The cat runs its own life through a behavior system driven by its stats. You influence it by caring for it (feeding, affection, grooming, training, play), by where you put it, and by what you buy for it. Minigames are how you earn coins and give it focused stat boosts.

```mermaid
flowchart LR
    player(("Player")) -- "Menu2: feed, pet,<br/>groom, train, play" --> actions["Care actions"]
    player -- "Menu1: minigames" --> games["Minigames"]
    player -- "Menu1: store" --> store["Store"]
    player -- "Menu1: locations,<br/>vacations" --> where["Where the pet is"]

    actions --> stats[("Pet stats<br/>(18 + sickness)")]
    games --> stats
    games --> coins[("Coins")]
    coins --> store
    store --> inv[("Inventory<br/>food, toys, medicine,<br/>pots, seeds")]
    inv --> actions

    world["World:<br/>time, weather,<br/>plants, Wi-Fi 'home'"] --> stats
    where --> world

    stats --> ai{"Behavior AI"}
    world --> ai
    ai -- "behaviors change stats<br/>when they finish" --> stats
    ai --> screen["What you see the cat do"]
    ai -- "hunting can find coins" --> coins
```

## Controls

| Button | In a location | In menus | In minigames |
| --- | --- | --- | --- |
| D-pad | Pan the camera (Left/Right) | Move selection | Game controls |
| A | Interact / confirm | Select | Game action |
| B | Back / cancel | Back | Varies |
| Menu1 | Open the main menu | Close | Open the main menu (how you leave a game) |
| Menu2 | Open the care menu for this location | Close | - |

Desktop keys: arrows, `A`, `S` (B), `Q` (Menu1), `W` (Menu2), `Esc` saves and quits.

## Screens and navigation

```mermaid
stateDiagram-v2
    [*] --> Adoption: no save
    [*] --> Inside: save found
    Adoption --> Inside: pet chosen and named

    state "Home locations" as Home {
        Inside --> Outside: cat wanders, or Locations menu
        Outside --> Treehouse
        Inside --> Bedroom
        Inside --> Kitchen
    }

    Home --> MainMenu: Menu1 (overlay, pet keeps living)
    MainMenu --> Home: B / Menu1 (close)
    MainMenu --> Stats
    MainMenu --> Forecast
    MainMenu --> Store
    MainMenu --> Minigame: Minigames submenu
    MainMenu --> Social: playdates over ESP-NOW
    Store --> Vacation: buy a trip
    MainMenu --> Debug: debug submenu

    Home --> CareMenu: Menu2
    CareMenu --> Home: action chosen

    Minigame --> MainMenu: Menu1 (rewards applied on exit)
    Vacation --> Home: "Go home"
```

Changing scenes always plays a short scanline fade. The swap happens while the screen is black.

### Adoption

The first boot with no save shows four adoptable pets. Each one has a different gender and personality seed, which sets the starting values of the five "personality" stats (courage, loyalty, mischievousness, curiosity, sociability).

![Adoption screen](images/adoption.png)

### Home locations

Five places the cat lives in. Each is a scrolling world wider than the screen; the camera follows the cat unless you pan it. The cat can walk between locations on its own (and will run inside if it starts raining).

| Living room (start scene) | Kitchen |
| --- | --- |
| ![Living room](images/inside.png) | ![Kitchen](images/kitchen.png) |
| **Bedroom** | **Outside (back yard)** |
| ![Bedroom](images/bedroom.png) | ![Outside](images/outside.png) |
| **Treehouse** (only reachable from Outside) | **Vacation: beach** |
| ![Treehouse](images/treehouse.png) | ![Beach vacation](images/vacation-beach.png) |

Outdoor scenes show the live sky: sun or moon by time of day, clouds, rain, snow, lightning, stars and meteor showers.

### Menus

**Menu1** opens the main menu as an overlay. The cat keeps going about its business underneath. **Menu2** in a location opens the care menu (Affection, Train, Feed, Play, and Gardening where there are plants).

| Main menu (Menu1) | Care menu (Menu2) |
| --- | --- |
| ![Main menu](images/main-menu.png) | ![Care menu](images/context-menu.png) |

Care menu tree:

```mermaid
flowchart LR
    m2["Menu2"] --> aff["Affection"] --> a1["Pets / Scratch / Kiss /<br/>Psst psst / Groom"]
    m2 --> train["Train"] --> t1["Intelligence / Behavior /<br/>Fitness / Sociability"]
    m2 --> feed["Feed"] --> f1["Meals / Snacks / Medicine"]
    m2 --> play["Play"] --> p1["Hand or owned toys"]
    m2 --> gard["Gardening"] --> g1["Tend / Place pot / Plant seed"]
```

Each care action interrupts whatever the cat is doing and starts a matching behavior (for example Feed starts `Eating`, Scratch starts `Affection`). The stat change is applied when that behavior finishes.

### Info screens

| Pet stats | Forecast | Store |
| --- | --- | --- |
| ![Stats](images/stats.png) | ![Forecast](images/forecast.png) | ![Store](images/store.png) |

- **Pet stats** shows one tile per stat, with a bar under each icon.
- **Forecast** is deterministic: the weather system can compute future weather from a seed, so the forecast is always right.
- **Store** sells food, snacks, toys, garden supplies, medicine, services and vacations for coins.

## How the pet decides what to do

The cat is always running exactly one **behavior** (29 in total: sleeping, napping, hunting, zoomies, kneading, sulking, mischief, and so on). Each behavior has a duration, a pose and animation, and a set of stat changes applied when it completes.

```mermaid
flowchart TD
    run["Current behavior runs<br/>(animation, movement)"] --> done{"Finished or<br/>interrupted?"}
    done -- "player action<br/>(Menu2)" --> trig["Start requested behavior now"]
    done -- completed --> bonus["Apply completion bonus<br/>(stat changes scaled by progress)<br/>record in recent history"]

    bonus --> shelter{"Outdoors in<br/>rain / snow / storm?"}
    shelter -- "70-90% roll<br/>(100% if sick)" --> goin["GoTo edge, then move<br/>to Inside (or Outside<br/>from the Treehouse)"]
    shelter -- no --> chain{"Behavior chains<br/>into a next one?"}
    chain -- "yes (and sickness<br/>doesn't block it)" --> next["Start chained behavior<br/>e.g. Hunting -> Eating"]
    chain -- no --> greet{"Player just woke<br/>the device?"}
    greet -- "yes, and pet has<br/>unmet needs or is happy" --> voc["Vocalizing (greeting)"]
    greet -- no --> auto["auto_select()"]

    auto --> bed{"Sleeping / Napping<br/>and there's a cat bed?"}
    next --> bed
    bed -- "60% if far away" --> walk["GoTo bed first,<br/>then sleep"]
    bed -- no --> run
    walk --> run
    goin --> run
    voc --> run
    trig --> run
```

### `auto_select()`: picking the next behavior

```mermaid
flowchart TD
    s(["auto_select"]) --> sh{"Need shelter?"}
    sh -- yes --> goShelter["Go indoors"]
    sh -- no --> mea{"Random meander roll<br/>20% healthy -> 2% very sick"}
    mea -- hit --> meander["Meandering"]
    mea -- miss --> exit{"Wants to wander<br/>to another room?"}
    exit -- yes --> goRoom["GoTo, then change scene"]
    exit -- no --> ser{"High serenity roll<br/>(serenity - 25) / 150"}
    ser -- hit --> idle["Idle"]
    ser -- miss --> cand["For each candidate behavior:<br/>skip if sickness blocks it<br/>skip if can_trigger() is false"]
    cand --> prio["priority = behavior's own priority(ctx)<br/>(LOWER = more likely)<br/>+ recency penalty if done recently"]
    prio --> bin["Round priorities up to the nearest 10,<br/>keep only the lowest bin"]
    bin --> pick["Pick one at random from that bin"]
    pick --> toy{"Playing?"}
    toy -- "owns a solo toy" --> solo["Play with a random owned toy"]
    toy -- no --> chosen["Start chosen behavior"]
```

Each behavior's `priority()` reads the stats. A tired cat gives `Sleeping` a low (good) number; a hungry cat makes `Vocalizing` about food likely; high playfulness favors `Zoomies` and `Playing`. The recency penalty (+50 for the most recent, then +40, +30...) stops the cat from repeating itself.

## Stats

The pet has 18 stats on a 0-100 scale, plus `sickness` (0-10).

```mermaid
flowchart TB
    subgraph rapid["Rapid (change ~daily)"]
        fullness & energy & comfort & playfulness & focus
    end
    subgraph medium["Medium (~weekly)"]
        fulfillment & cleanliness & intelligence & maturity & affection
    end
    subgraph slow["Slow (~monthly)"]
        fitness & serenity
    end
    subgraph slowest["Personality (very slow, seeded at adoption)"]
        courage & loyalty & mischievousness & curiosity & sociability
    end

    health(["health<br/>(computed, never set directly)"])
    fullness -- "25%" --> health
    fitness -- "20%" --> health
    energy -- "20%" --> health
    cleanliness -- "15%" --> health
    comfort -- "5%" --> health
    affection -- "5%" --> health
    others["fulfillment, focus,<br/>intelligence, playfulness<br/>2.5% each"] --> health
```

How stats move:

- There's no global "hunger timer". Stats change when behaviors finish: even `Idle` costs a little fullness, energy, comfort and cleanliness, while `Sleeping` restores energy, `Eating` restores fullness, and so on.
- All changes go through `apply_stat_changes`, which damps them near the ends: gains get smaller as a stat approaches 100, losses get smaller near 0, and stats below 25 get up to a 2x catch-up boost on gains.
- Minigames apply their own changes when you leave them (for example Zoomies: +fitness, +playfulness, +sociability, −energy, −fullness).
- Plants in the room, being "at home" (familiar Wi-Fi), weather and temperature all feed into behavior choices and comfort.

## Time, seasons and weather

Game time runs at **1 game minute per 15 real seconds**: an in-game hour is 15 real minutes, and an in-game day is 6 real hours. The debug menu's "Time Speed" multiplies this for testing.

```mermaid
flowchart LR
    dt["Frame dt<br/>x time_speed"] --> clock["Clock<br/>hours:minutes"]
    clock -- "every 24 h" --> day["day_number"]
    day --> season["Season<br/>Winter / Spring /<br/>Summer / Fall"]
    day --> moon["Moon phase<br/>(8 phases)"]
    clock -- "every hour" --> temp["Temperature<br/>(day, season, hour,<br/>weather, pet seed)"]
    clock --> weather["Weather system"]
    season --> weather
    weather --> sky["Sky rendering"]
    weather --> sick["Sickness if outdoors"]
    weather --> shelter["Shelter-seeking"]
```

Weather is a seeded Markov chain. Each state lasts a random duration, then moves to a neighbor state; seasons change which neighbors are allowed (snow belongs to the cold seasons), and meteor showers roll by season.

```mermaid
stateDiagram-v2
    Clear --> Cloudy
    Clear --> Windy
    Cloudy --> Clear
    Cloudy --> Overcast
    Cloudy --> Windy
    Overcast --> Cloudy
    Overcast --> Rain
    Overcast --> Windy
    Windy --> Clear
    Windy --> Cloudy
    Windy --> Overcast
    Rain --> Overcast
    Rain --> Storm
    Storm --> Rain
    Storm --> Overcast
    Snow --> Cloudy
    Snow --> Overcast
```

## Sickness

```mermaid
flowchart LR
    subgraph causes["Gains while awake (per real minute)"]
        storm["Outdoors in a storm: +0.08"]
        rain["Outdoors in rain / snow: +0.04"]
        hungry["Fullness < 10: +0.03"]
        dirty["Cleanliness < 15: +0.03"]
        snacks["4+ snacks in a row"]
    end
    causes --> sick[("sickness 0-10")]
    sick -- ">= 2" --> block1["Blocks zoomies, mischief, hunting,<br/>investigating, observing, playing"]
    sick -- ">= 5" --> block2["Blocks most other active behaviors"]
    sick -- ">= 2" --> shelter["Always seeks shelter from bad weather"]
    sick --> lessMeander["Less meandering, more resting"]
    rest["Each sleep / nap: −1.0<br/>(−3.0 with medicine)"] --> sick
```

Sleeping and napping don't count toward sickness gain, and are how the pet recovers. Medicine is bought in the store and given from Feed.

## Minigames and coins

Minigames are separate scenes picked from Menu1 → Minigames. Rewards (coins and stat changes) are applied in the scene's `exit()`, so you collect them when you leave with Menu1.

| Zoomies | Breakout | Snake |
| --- | --- | --- |
| ![Zoomies](images/zoomies.png) | ![Breakout](images/breakout.png) | ![Snake](images/snake.png) |
| **Hunter (platformer)** | **Maze** | **Mini Golf** |
| ![Hunter](images/platformer.png) | ![Maze](images/maze.png) | ![Mini Golf](images/minigolf.png) |

Full list: Zoomies, Breakout, Snake, Hunter, Memory, Maze, TicTacToe, Reversi, Purrimeter, Yarnball, Lucky Landing, Balancing Act, Hanjie, Lights Out, Pipes, Sunny Spot, Herding, Meowltiply, Sleeping Dogs, Sudokat, Mini Golf, Laser Bounce.

```mermaid
sequenceDiagram
    actor P as Player
    participant L as Location scene
    participant M as Main menu overlay
    participant G as Minigame scene
    participant C as GameContext

    P->>L: Menu1
    L->>M: open overlay (pet keeps living)
    P->>M: Minigames > Zoomies
    M->>G: fade, swap scene, enter()
    loop each frame
        P->>G: buttons
        G->>G: update score
    end
    P->>G: Menu1
    G->>M: open overlay
    P->>M: Locations > Living Room
    M->>G: fade, exit()
    G->>C: apply stat changes, coins += reward
    M->>L: swap scene, enter()
```

Coins also come from the cat itself: sometimes a hunting cat finds a few.

## Sleep and power

```mermaid
stateDiagram-v2
    [*] --> Awake
    Awake --> FadingOut: 15 min without a button press
    FadingOut --> ScreenOff: transition midpoint
    state ScreenOff {
        [*] --> Ticking
        Ticking --> Ticking: game updates at 2 FPS<br/>(time, stats, behaviors continue)
    }
    ScreenOff --> Waking: any button (press is swallowed)
    Waking --> Awake: fade in, pet may greet you
    Awake --> DeepSleep: Debug > Power > Deep sleep
    DeepSleep --> [*]: A/B/Menu wakes the chip<br/>(full reset, save reloaded)
```

While the screen is off the game keeps running at 2 FPS, so the cat keeps living. Behaviors that would change the scene are ignored until wake. On wake, a sleeping cat winds down quickly and may greet you by vocalizing if it has needs or is in a good mood.

## Home, vacations and playdates

- **Home comfort:** once an hour (hidden behind a scene fade) the device scans Wi-Fi. Networks seen often become "familiar". When a familiar network is visible the pet is "at home", which makes it calmer and sleep better; somewhere unfamiliar it gets restless.
- **Vacations:** park, forest, aquarium and beach. Bought in the store, they're special locations with their own art; the pet eventually wants to go home ("Go home" appears in the care menu).
- **Playdates:** in the Social screen two devices find each other over ESP-NOW. The visiting cat appears on both screens, the cats mirror and react to each other's behaviors, and they build up friendship over time.
- **Gardening:** pots and seeds from the store are placed in a location and grow in stages over real days (1 in-game hour = 15 real minutes). They need watering and fertilizer, and healthy plants raise the pet's satisfaction in that room.
