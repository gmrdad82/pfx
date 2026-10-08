# Input

`pfx-input` (`crates/input`) is the engine's input crate. A game uses it for the keyboard, the mouse and controllers, all behind one action map. It also provides controller-family glyphs, rumble, and input recording. No other engine crate depends on it. It opens no window and reads no clock. Everything time-based counts fixed game steps, which the game drives.

## The step

A game builds an `Input` from its `Actions` and the length of its fixed step in microseconds. Each step it does two things:

- it feeds events with `Input::feed`: winit's through `winit::event`, and the pads' through a `Backend`;
- it calls `Input::step`, which turns those events into action states.

`Input::update(&mut backend)` does both for a backend. It polls the backend, feeds the events, steps, and sends the step's rumble commands back to the backend.

`Step` reports three things:

- the step index;
- `changed`, the new last-used device when it changed this step;
- `motors`, the rumble commands.

## Devices

| Device | Source | Feature |
|---|---|---|
| Keyboard | `winit::event(&WindowEvent)`: physical keys (`Key`), layout-independent | `winit` (default) |
| Mouse | buttons, wheel, `CursorMoved`, `CursorLeft` | `winit` |
| Gamepads | `pads::Gilrs`, gilrs 0.11.2 pinned exactly, with force feedback | `gilrs` (default) |
| Steam Input | a backend from the Steam crate, behind that crate's feature | none here |

Key events with `repeat` set are dropped, because the action map makes its own repeats. A synthetic press (winit sends one for keys held while the window gains focus) is dropped too. A synthetic release is kept. `WindowEvent::Focused(false)` becomes `InputEvent::FocusLost`, which releases every held key and mouse button.

`Backend` has two methods: `poll` (append events) and `set_motors` (play a `MotorCommand`). `Script` is an in-memory backend for tests and tools. `Player` replays a `Recording`.

A Steam Input backend implements `Backend` in the Steam crate. That way this crate never links the Steamworks SDK. Such a backend reports each controller with these:

- `PadInfo::resolves_actions` set;
- its family from `Family::from_steam_input_type` (the `ESteamInputType` value);
- `InputEvent::PadAction { action, value }` for each action's digital or analog state;
- `InputEvent::PadOrigins { action, controls }` for the buttons Steam's configurator bound to that action.

For such a pad, the gamepad bindings in the map are skipped. Its action values and origins are used instead, so a player who rebinds in Steam's configurator sees the new buttons in the game's prompts. `set_motors` maps to `TriggerVibration`, with the low-frequency motor on the left and the high-frequency motor on the right.

Under Steam Input, gilrs sees Steam's virtual pad (`28DE:11FF`) and reports it as `Family::Generic`. The Steam backend is the one that knows the real controller.

### Steam Input wins over gilrs

While a Steam Input backend is active, gilrs must not report the pads Steam already reports, or every press arrives twice and a DualSense shows Generic or Xbox glyphs. This is a hook, not detection: `Backend::steam_input(active)` has a default that does nothing, and `pads::Gilrs` overrides it. The Steam crate calls it on the backend it runs beside, with `true` once its Steam Input backend is up and `false` when it shuts down. Detection alone can't work, because Steam's virtual pad is also the only way to read a controller when the game hasn't turned Steam Input on.

The rule lives in `SteamPrecedence`, which `Gilrs` runs every event through. While Steam Input is active, it hides a pad when:

- its ids are `28DE:11FF`, Steam's virtual pad, or its name contains `Steam Virtual Gamepad`;
- its ids are on the list of physical devices Steam Input owns, given with `owned(list)` in SDL's format (`0x054C/0x0CE6,0x28DE/0x11FF`). Steam writes that list into `SDL_GAMECONTROLLER_IGNORE_DEVICES` for the games it launches, and `SteamPrecedence::from_env()` reads it. On a Linux desktop, this is what hides the raw DualSense that gilrs would otherwise read beside Steam's copy;
- with `xinput(true)`, it is an Xbox 360 pad (`045E:028E`) or has no ids. That is how Steam's virtual Xbox pads look through XInput on Windows and under Proton. It is off by default, because a real Xbox 360 pad that Steam Input doesn't manage would be hidden with it.

A pad that resolves its own actions (`PadInfo::resolves_actions`, the Steam backend's) is never hidden. A hidden pad's buttons, axes and disconnect are dropped, and `set_motors` on it returns false. Turning Steam Input on sends `PadDisconnected` for the pads it now hides, and turning it off sends `PadConnected` for them again. Without the hook, nothing is hidden. Pass a configured precedence with `Gilrs::new()?.with_steam(SteamPrecedence::from_env())`, adding `.xinput(true)` when the game knows every pad goes through Steam Input.

## Actions

An `ActionSpec` names an action and gives its kind:

- `digital`: press, release, held and repeat;
- `axis`: one value from -1 to 1;
- `stick`: two values, with y up.

A spec also has a `context`, a dead zone (0.2 by default for analog actions, 0 for digital ones), an optional `repeat(delay, interval)` in steps, and default bindings for each device kind (`key` for keyboard and mouse, `pad` for gamepads).

| Binding | Reads |
|---|---|
| `Press(source)` | one source, from 0 to 1. A key is 0 or 1. A trigger is analog |
| `Axis { negative, positive }` | positive minus negative |
| `PadAxis(axis)` | one stick axis, from -1 to 1 |
| `Stick { up, down, left, right }` | four sources as a vector, clamped to length 1 (WASD, arrows, the d-pad) |
| `PadStick(stick)` | a whole stick |

A `Source` is a key, a mouse button, a wheel direction, a pad button, or half of a stick axis (`Axis(axis, sign)`). Triggers are pad buttons with analog values.

Each step, every action gets an `ActionState`:

- `held`, `pressed`, `released` and `repeated`;
- `held_steps`;
- `value` (the analog magnitude, or the axis value);
- `vector`.

`fired()` is `pressed || repeated`, which suits menu navigation. With `repeat(20, 6)`, a held direction fires on the step it is pressed, then at 20, 26, 32 and so on.

The bindings of every device are combined. A digital action is held when any of its sources reads at least 0.5. An analog action takes the binding with the largest magnitude. A press and a release that both arrive inside one step still count: the action is pressed on that step and released on the next. A key that is still down does not count as a tap, so opposing keys on one axis cancel.

Dead zones are rescaled. An axis reads 0 up to the dead zone, then rises linearly to 1. A stick's dead zone is radial on its length, and the direction is kept. The dead zone applies to every analog read, keys included, where it changes nothing because a key reads 0 or 1.

`Input::action(name)` returns the state by name. `Input::state(id)` returns it by `ActionId`, from `Actions::id`.

## The binding map

`BindingMap` is the part the game saves. It has two sets, `keyboard` (keyboard and mouse) and `gamepad`, each a map from action name to its bindings, and it serializes with serde. `BindingMap::defaults` builds it from the specs. When a saved map is loaded, `Input::set_bindings` fits it to the current actions: retired actions are dropped and new actions get their defaults.

Keyboard rebinding happens in the game's settings, and goes through `rebind_key(actions, name, slot, source, on_conflict)`. A `Slot` names the set, the index of the binding, and the part of a composite binding (for `Stick`, the parts are up, down, left and right). Only keys and mouse buttons are accepted.

A conflict is the same source already bound to another slot in the same context. `pause` and `back` can both be Escape because play and menus are different contexts. On a conflict, the game picks what happens:

| `OnConflict` | Effect |
|---|---|
| `Refuse` | `RebindError::Conflicts` lists every conflict, and the map is unchanged |
| `Swap` | each conflicting slot gets the old source of the slot being rebound |
| `Replace` | each conflicting binding is removed. A conflict inside the binding being edited is swapped instead |

`conflicts(...)` lists the conflicts without changing anything, so a settings screen can show them first. `reset` restores one action's defaults. `Input::edit_bindings` edits the map in place and refits it.

Controllers are rebound through Steam Input's configurator, not here. The gamepad set can still be edited from code with `rebind`.

## The pointer

`Input::set_viewport(viewport)` takes the window layer's `Viewport`. A `PointerMoved` in window pixels goes through `Viewport::pointer_to_layout` and `pointer_inside` when it is fed. It becomes a `PointerAt` in layout units (1920×1080), and `inside` is false in the letterbox bars. Without a viewport, window pixels pass through unchanged.

The pointer is the mouse's alone. A controller never moves it. In play the mouse clicks while the controller steps. In menus the controller moves focus while the mouse hovers.

## The last-used device

`Input::active()` is an `Active` with three fields: `kind` (Keyboard, Mouse or Gamepad), `pad`, and the pad's `family`. These things make a device the active one:

| Device | What counts |
|---|---|
| Keyboard | a key press |
| Mouse | a button press, the wheel, or moving at least 4 layout units from where the pointer was when another device took over |
| Gamepad | a button rising past 0.5, a stick axis passing 0.5 (stick drift does not count), or a Steam action reaching 0.5 |

`Step::changed` carries the new `Active` on the step it changes. Disconnecting the active pad switches back to the keyboard.

`Input::prompt(name)` returns the `Control`s to show for an action on the active device:

- with the keyboard active, the first binding made only of keys;
- with the mouse active, the first binding with a mouse source;
- with a gamepad active, the first gamepad binding, or Steam's origins.

`Input::prompt_family()` is the active family. While the player is on the keyboard, it is the family of the last pad used, and otherwise `Generic`.

## Controller families

`Family` is DualSense (the DualSense Edge included), DualShock 4, Xbox (360, One and Series, including the Elites), Steam Controller, Steam Deck, Switch Pro (Joy-Cons included), or Generic. `Family::from_ids(vendor, product)` reads the `family::PADS` table, keyed on the USB ids gilrs reports.

An unknown product falls back to its vendor: Sony (`054C`) is DualShock 4, Microsoft (`045E`) is Xbox, Valve (`28DE`) is Steam Controller, and Nintendo (`057E`) is Switch Pro. Any other vendor is Generic. `Family::from_steam_input_type` maps Steam Input's controller type.

Buttons are positional, the way gilrs and Steam Input report them. `South` is the bottom face button on every pad, A on an Xbox pad and B on a Switch Pro.

## Glyphs

`glyph(family, control)` returns a `Glyph`. `GlyphAtlas::build()` draws all of them, and `GlyphAtlas::get(glyph)` or `cell(family, control)` returns the glyph's `GlyphCell`.

| Control | Xbox | DualSense, DualShock 4 | Switch Pro | Steam Controller | Steam Deck | Generic |
|---|---|---|---|---|---|---|
| South, East, West, North | A B X Y in a disc | cross, circle, square, triangle in a disc | B A Y X | A B X Y | A B X Y | four dots, the pressed position filled |
| Bumpers | LB RB | L1 R1 | L R | LB RB | L1 R1 | LB RB |
| Triggers | LT RT | L2 R2 | ZL ZR | LT RT | L2 R2 | LT RT |
| Stick clicks | LS RS | L3 R3 | LS RS | LS RS | L3 R3 | LS RS |
| Menu | three bars | three bars in a pill | plus | forward triangle | three bars | three bars |
| View | two squares | three rays in a pill | minus | back triangle | two squares | two squares |
| Grips | P3 P1 P4 P2 | L4 R4 L5 R5 | L4 R4 L5 R5 | LG RG | L4 R4 L5 R5 | L4 R4 L5 R5 |

Every family also has these:

- sticks (a ring with L or R), the d-pad (whole and each direction), Guide (a house in a disc) and the touchpad;
- Misc: a share arrow on Xbox, a capture square on Switch, and M elsewhere.

Keys are blank keycaps, and the arrow keys show arrows. The label is text drawn on the cap (see Keycap labels), so a layout's letters need no atlas rebuild. Caps come in six widths, 48, 64, 80, 96, 128 and 160 (`keycap::KEYCAP_WIDTHS`). `glyph` picks one from the US label's length, and `KeycapLabel` picks one from the measured label. The mouse glyphs are a mouse body with the pressed button filled, the wheel with an arrow for each wheel direction, and four arrows for mouse movement.

All glyphs are original geometric shapes defined as vectors in code (`glyph::shape`), with pad labels from a 5×7 pixel alphabet traced by marching squares. They do not reproduce any company's logo or button art. The cross, circle, square and triangle are plain shapes. A mark is knocked out of a filled shape. A stick is a ring with a filled label, and a keycap is a blank ring.

Every contour is oriented by its nesting depth before it is written (`Shape::oriented`). Depth is found from containment, and each contour's parent is the smallest contour around it. Outer contours are clockwise in font units, as TrueType expects, and each hole winds opposite to its parent, so the glyphs fill the same under the nonzero rule the text crate uses as under even-odd. A test checks the winding against every parent, and another samples every glyph on a half-pixel grid and compares nonzero with even-odd.

The atlas goes through the text crate's own MSDF generator. `glyph::font::build` writes the shapes into an in-memory TrueType font (`glyf` contours, private-use code points from U+E000), and `TextEngine::layout` with `Representation::Msdf` rasterizes and packs that font. The result is decoded with `pfx_text::MSDF_WGSL` or `msdf_coverage`.

The atlas has these properties:

- 1024×512 pixels, RGB, with a mip chain;
- 64 pixels per em, and 48 pixels for a glyph box's height;
- a distance range of 8 pixels.

The box is 48 wide, except bumpers and the touchpad (64), grips (56), and keycaps (their width). For a glyph shown `h` pixels tall, scale by `h / 48`. Place `size × scale` at `offset × scale` from the box's top left, and sample `uv`.

`GlyphAtlas::digest` is SHA-256 over the base level and every cell. `glyph::ATLAS_DIGEST` pins it. A change to any shape, the label alphabet or the text crate's MSDF generator changes the digest. Re-pin it only after checking the new atlas.

## Glyph style

A game that shows controller prompts needs only the style of the controller in the player's hand. `GlyphStyle` has two values: `PlayStation` and `Standard`.

- `Family::style()` maps a family: DualSense (the Edge included) and DualShock 4 are `PlayStation`; every other family is `Standard`. A Switch Pro (Switch 2 Pro included) is only supported through Steam, which presents it as a standard pad, so it reads as `Standard` with no Nintendo swap: South is A, East is B. The Steam Deck's own L1/R1 labels are not a third style.
- `Input::glyph_style()` is the style of the pad the player last used, kept while the player is on the keyboard or the mouse, and `Standard` before any pad was used. `Step::style` carries the new style on the step it changes (a different controller picked up, or the pad disconnected), so prompts switch live. It is separate from `Step::changed`, which also fires for the keyboard and mouse.
- `glyph_for(style, control)` returns the `Glyph` of a control in that style, drawn from the same atlas as `glyph`, crisp at any size. PlayStation shows cross, circle, square and triangle with L1 L2 R1 R2, L3 R3, the touchpad, options (bars in a pill) and create (rays in a pill). Standard shows A B X Y with LB LT RB RT, LS RS, menu and view. Keys and the mouse look the same in both.

What a game does:

1. Draw each prompt with `glyph_for(input.glyph_style(), control)`, taking the controls from `Input::prompt(name)`, and look the glyph up with `GlyphAtlas::get`.
2. Redraw its help, messages and tutorial prompts when `Step::style` is `Some`, and when `Step::changed` moves between a pad and the keyboard or mouse.
3. Decide for itself where prompts appear. The engine has no glyph registry, no Steam glyph API and no glyph packs.

## Keycap labels

Bindings stay by physical key position (winit's `PhysicalKey`, `Key` here), so a French player's WASD is ZQSD on the keycaps without any rebinding. The label shown on a key comes from the player's active layout: AZERTY shows A where QWERTY shows Q, and QWERTZ shows Z where QWERTY shows Y.

`KeyLabels` is the label cache. `label(key)` returns the cap's text. Only letters and the punctuation keys follow the layout (`Key::follows_layout`). Digits, F keys, the numpad and named keys keep their US labels, unless the game renames one for its language with `rename(key, name)` (Maj, Strg, Échap). Labels are capitals, one character for one: `ù` becomes `Ù`, and `ß` stays `ß`.

Labels come from a `LayoutSource`, which has two methods: `layout()`, an id of the active layout, and `label(key)`, the key's unshifted character. `refresh(&mut source)` asks for the id and reads every layout key again only when the id changed, so a game calls it once a frame on the window's thread for the cost of one call. It returns true when a label changed.

| Platform | Source |
|---|---|
| Windows, and the Windows build under Proton | `WindowsLayout`: the id is `GetKeyboardLayout(0)`; a key's set-1 scancode (`layout::scancode`) goes through `MapVirtualKeyExW` to a virtual key, then to its unshifted character, with a dead key's flag cleared. A key with no character falls back to `GetKeyNameTextW`. windows-sys 0.61.2, pinned, behind `cfg(windows)` |
| Linux dev builds | learned from key events: `winit::learn(&mut labels, &event)` reads `key_without_modifiers` from winit's modifier supplement on each press, a dead key included |
| anything else | US (`Us`, or nothing at all) |

winit 0.30 doesn't surface `WM_INPUTLANGCHANGE`, and the message is sent to the window procedure rather than posted, so winit's message hook never sees it. Comparing the layout id each frame catches the same change. On Linux there is no layout event, so learning refreshes itself: when a learned key reports a different label, the layout has changed, so every learned label is dropped and the new one kept. Until the player presses a key under the new layout, that key keeps its old label.

`KeycapLabel` draws a label through the text crate's own MSDF path, with the game's font and `TextEngine`:

- `KeycapLabel::key(engine, face, labels, key)`, or `fit(engine, face, text)` for any text, measures the label with `layout_spans` at 26 units for one character and 18 for a word, then picks the narrowest cap whose inside (the width less 10 on each side) holds it. A label too long for the widest cap is shrunk to fit. An arrow key gets its arrow glyph and no text;
- `key_with_floor` and `fit_with_floor` take a minimum label size in glyph-box units: a label that would shrink under it is held at the floor (or at its own em when that is smaller) and the cap widens past 160 instead, to the label's width plus the two insets (`keycap::widened`). A floor of 0 is `key` and `fit`. `GlyphAtlas::pieces(glyph)` draws such a cap from the 160 cell: its left half, its right half moved out by the extra width, and tiles of its plain middle between them, all with the cap's own `box_size`; any glyph in the atlas is one piece, and a keycap narrower than 160 that is not in the atlas is `None`;
- the result has the cap `glyph`, the `text`, its `size` and its `origin` (the block's top left), all in glyph-box units (48 tall). The baseline sits so a capital's middle is the cap's middle;
- `place(engine, face, color, origin, height, placement)` scales that to a cap drawn `height` tall at `origin` and calls `place_spans`. The quads land on the engine's shared atlas pages beside the game's other text.

The game draws the cap from the `GlyphAtlas` and then the quads, in the same colour.

## Rumble

`Rumble::new(strength, duration_ms)` drives both motors. `Rumble::motors(low, high, duration_ms)` sets the low-frequency (strong) motor and the high-frequency (weak) motor separately. `attack(ms)` and `release(ms)` ramp the level in and out.

`Input::rumble` plays on the active pad, and only while a gamepad is the active device. `rumble_pad` targets one pad. Nothing plays when the intensity is 0 or the pad has no motors.

Overlapping events take the stronger value on each motor. A one-motor pad gets the stronger of the two.

The player's setting runs from 0 to 100% (`set_rumble_intensity`). It scales every level, which is then quantized to `u16`. A command is sent only when a motor's value changes. Its `hold_ms` is the time left on the shortest event plus one step, so a stalled game stops the motors by itself.

`pads::Gilrs` plays each command as a gilrs force-feedback effect, a strong and a weak base effect, for `hold_ms`, and replaces the previous effect. gilrs force feedback works on Linux evdev and on Windows. Under Steam Input, the Steam backend's `TriggerVibration` takes over.

## Recording and replay

`Input::start_recording` starts recording at a step boundary, between `step` and the next `feed`. A `Recording` holds these:

- `step_us`;
- a `Start` snapshot: the events that rebuild every connected pad and held control, the action states, the active device, the last pad, and the pointer anchor;
- every step's events, after normalization, so the pointer is already in layout units.

The recording serializes with serde, and serde_json round-trips each `f32` exactly.

To replay, build a fresh `Input` with the same actions and bindings, call `begin_replay(&recording)`, then call `update(&mut recording.player())` once per step. Every action state, the pointer, the active device and the change events come back bit for bit: the test compares the `f32` bits over 600 steps of random input. That lets pfx shots and replays drive a game without a device. Rumble commands during a replay go to `Player::motors` and are not played.

## Real-hardware check (pending)

The tests use fake layouts and fake pads; nothing here has been checked on hardware yet. It needs a pad, a window and Steam. To check, with a pad and Proton available:

1. Build the game's Windows build with Steam Input on (the Steam crate's backend calling `steam_input(true)`), and run it from Steam, on a Steam Deck or under Proton on the desk.
2. Connect a DualSense by USB, then by Bluetooth. The prompts must show cross, circle, square and triangle, never A B X Y or the four dots. Press each face button once: each action fires once, not twice.
3. In Steam's controller configurator, swap two buttons for the game. Without restarting, the prompts must show the swapped buttons.
4. Trigger a rumble: it plays on the DualSense through Steam Input.
5. Turn Steam Input off for the game: the pad still works through gilrs, as Generic or by its ids.
6. Switch Windows (or the Deck's desktop) to French AZERTY, then German QWERTZ, while the game runs. Within a frame, the keycap prompts show A where QWERTY shows Q, Z where it shows Y, and Ö, Ü and ß on their keys, with the bindings themselves unchanged.

