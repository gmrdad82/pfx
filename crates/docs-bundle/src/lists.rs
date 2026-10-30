pub const GUIDES: &[(&str, u32, Option<&str>)] = &[
    (
        "install",
        99,
        Some(
            "Install pfx on Linux, Windows or macOS from a GitHub Release, check it against SHA256SUMS, and open .pitofx files in pfx's window.",
        ),
    ),
    (
        "migrate-3",
        100,
        Some(
            "Move a game from pfx 2.6.0 to 3.0.0: the crates, what moved in, the renames, converting scenes and recipes, one binary, the window and Play.",
        ),
    ),
    (
        "first-game",
        101,
        Some(
            "Scaffold a game with pfx new, play it with cargo run, open it in the editor and measure it with pfx bench.",
        ),
    ),
    (
        "projects",
        102,
        Some(
            "A game on pfx is its own repo and one binary: what pfx new scaffolds, the project.toml, and how a game plays and opens in the editor.",
        ),
    ),
    (
        "game",
        103,
        Some(
            "pfx-game, the one call a game's main makes: the window, pacing, input, sound, the fixed tick and the game's own launcher.",
        ),
    ),
    (
        "splash",
        103,
        Some(
            "The start-up splash every game on pfx opens with: what it shows, how it overlaps loading, its settings, and the pro key that hides it.",
        ),
    ),
    (
        "lockup",
        103,
        Some(
            "Draw pfx's Powered by lockup on a flat board in a credits or licences screen: the same artwork as the splash, whole, at any height and position.",
        ),
    ),
    ("scenes", 104, None),
    ("scene-crate", 104, None),
    ("scene-format", 105, None),
    ("scene-projects", 106, None),
    ("scene-check", 107, None),
    ("scene-check-buffer", 108, None),
    ("scene-resolve", 109, None),
    ("scene-edit", 110, None),
    ("scene-ids", 111, None),
    ("scene-migrate", 112, None),
    ("scene-speed", 113, None),
    ("scene-schema", 114, None),
    ("pfx-file", 115, None),
    ("recipes", 116, None),
    ("recipe-first-scene", 117, None),
    ("recipe-format", 118, None),
    ("recipe-build", 119, None),
    ("recipe-files", 120, None),
    ("recipe-documents", 121, None),
    ("recipe-gltf", 122, None),
    ("recipe-library", 123, None),
    ("recipe-render", 124, None),
    ("recipe-clips", 125, None),
    ("recipe-runtime", 126, None),
    ("recipe-noise", 127, None),
    ("asset-sources", 128, None),
    ("git", 129, None),
    ("cli-coverage", 130, None),
    ("play", 201, None),
    (
        "input",
        202,
        Some(
            "pfx-input: keyboard, mouse and controllers behind one action map, with controller glyphs, rumble and input recording.",
        ),
    ),
    ("sound", 203, None),
    ("physics", 204, None),
    ("characters", 205, None),
    ("camera-rigs", 206, None),
    (
        "animation",
        207,
        Some(
            "Skeletal animation from glTF, node animation, and sprite clips from an image atlas, all deterministic by tick, drawn live with GPU skinning and traced in the same pose.",
        ),
    ),
    ("screens", 208, None),
    (
        "window",
        209,
        Some(
            "pfx-gpu's window feature: the display mode, the present mode, the render size and the frame cap.",
        ),
    ),
    ("sim", 210, None),
    ("shots", 211, None),
    ("director", 212, None),
    ("journal", 213, None),
    ("numbers", 214, None),
    ("words", 215, None),
    ("easings", 216, None),
    ("mods", 217, None),
    (
        "film",
        218,
        Some(
            "pfx-film: scripted films rendered offline from a game at a fixed frame rate, with a camera, looks, captions, marks and a synthesised score, the same bytes every run.",
        ),
    ),
    (
        "live-renderer",
        301,
        Some(
            "How the live renderer draws a frame: its frame budget at 4K, passes, shadows, baked light, TAA, text, frames on demand and dynamic resolution.",
        ),
    ),
    ("benchmark-a-scene", 302, None),
    ("frame-stats", 303, None),
    ("pacing", 304, None),
    ("gpu-floor", 305, None),
    ("plates", 306, None),
    ("flat", 307, None),
    ("flat-styles", 308, None),
    ("flat-looks", 309, None),
    ("effects", 310, None),
    ("text", 311, None),
    ("materials", 312, None),
    ("render-a-still", 313, None),
    ("trace", 314, None),
    ("trace-lights", 315, None),
    ("trace-film", 316, None),
    ("denoise", 317, None),
    (
        "bake",
        318,
        Some(
            "pfx bake traces a scene's probe volumes, prerendered props, plates and detail maps on the GPU and writes versioned, checked artifacts the live renderer shows.",
        ),
    ),
    (
        "geom-assets",
        319,
        Some(
            "pfx-geom's asset geometry on the CPU: SVG marks as extruded, bevelled solids and icon parts fused through a distance field.",
        ),
    ),
    (
        "render-a-scene",
        320,
        Some(
            "pfx shot renders a scene file offline on the live renderer at a fixed clock: numbered frames, a manifest and a WAV for pcod, the same bytes every run.",
        ),
    ),
    ("particles", 321, None),
    ("alpha-master", 322, None),
    (
        "memory",
        323,
        Some(
            "How pfx plans a run's memory before it loads, refuses one past --memory-limit, keeps its own allocations under it, and reports the peak beside the plan.",
        ),
    ),
    ("jobs", 324, None),
    (
        "cache",
        325,
        Some(
            "The shared cache every pfx run and checkout reuses: compiled pipelines and bakes keyed by their inputs, checked on read, capped by last use, and pfx cache info and clean.",
        ),
    ),
    ("editor", 401, None),
    ("editor-crates", 402, None),
    ("play-modules", 403, None),
    (
        "editor-window",
        404,
        Some(
            "Arrange a recipe in pfx gui: open it, pick and move parts, change materials, set cameras and scrub the sun, every edit kept until Ctrl+S writes it into the .pitofx.",
        ),
    ),
    (
        "editor-recipes",
        405,
        Some(
            "How pfx gui writes a recipe: gestures, undo, outside changes, curves, the hour, the timeline, the picker, the start scene and the File menu.",
        ),
    ),
    ("editor-cameras", 406, None),
    ("editor-gizmos", 407, None),
    (
        "editor-shots",
        408,
        Some(
            "pfx gui drawn offscreen: one frame with --shot after a script, or a clip of the window playing a tape.",
        ),
    ),
    (
        "editor-rig",
        409,
        Some(
            "bin/rig: the live pfx gui window in a hidden compositor, for screenshots, keys and the pointer.",
        ),
    ),
    (
        "settings",
        410,
        Some(
            "One settings panel for pfx gui and a game's editor, a settings menu for games, and pfx settings: VRR, the frame rate, the quality, each costly effect and the licence.",
        ),
    ),
    (
        "windows",
        501,
        Some(
            "pfx on Windows: the tool's Windows build, and the one game build Steam ships, which Linux and the Steam Deck run through Proton.",
        ),
    ),
    ("steam", 502, None),
    ("wgpu30", 503, None),
    ("agnostic", 504, None),
    (
        "measured",
        505,
        Some(
            "How the numbers on the front page were measured: the live renderer and the 2D pass at 4K, the game systems, the size of an empty game, and pfx against Godot, Bevy, Unreal and Blender.",
        ),
    ),
];

pub type Pick = (&'static str, Option<&'static str>);

pub type Format = (&'static str, &'static str, &'static [Pick]);

pub const FORMATS: &[Format] = &[
    (
        "Scenes and projects",
        "Scenes are pfx-scene's TOML format; a project is a folder with a project.toml.",
        &[
            ("scenes", Some("The format is pfx-scene's")),
            ("projects", Some("A project")),
        ],
    ),
    (
        ".pitofx files",
        "pfx's native file: a chunk table, then checksummed chunks, each stored or LZ4 compressed.",
        &[("pfx-file", Some("The layout")), ("pfx-file", Some("Tags"))],
    ),
    (
        "Scene JSON Schemas",
        "A JSON Schema for every kind of scene file, generated from the format's types.",
        &[("scene-schema", None)],
    ),
    (
        "Material libraries",
        "One TOML file of named materials that both renderers and the bake share.",
        &[("materials", None)],
    ),
    (
        "Frame stats",
        "One JSON line per presented frame, and an end line.",
        &[
            ("frame-stats", Some("The line")),
            ("frame-stats", Some("The end line")),
        ],
    ),
    (
        "The bench summary",
        "The JSON line pfx bench prints last.",
        &[("frame-stats", Some("The summary"))],
    ),
    (
        "Input journals",
        "The compact binary input log a game ships and verifies headless.",
        &[("journal", Some("The bytes"))],
    ),
    (
        "Input recordings",
        "The engine's own JSON log of a session's input, which pfx bench --replay reads.",
        &[("input", Some("Recording and replay"))],
    ),
    (
        "Steam depots",
        "The manifest pfx steam stage reads and the files it writes.",
        &[
            ("steam", Some("steam.toml")),
            ("steam", Some("What gets written")),
        ],
    ),
    (
        "Baked detail atlases",
        "What pfx bake --detail writes and the engine loads.",
        &[("bake", Some("Files and manifest"))],
    ),
];

pub const SKIP: &[&str] = &["format", "formats"];

pub const MOVED: &[(&str, &str)] = &[
    ("install-and-first-use", "install.md"),
    ("build-and-test", "install.md#build-pfx-from-source"),
];

pub const FLAG_GROUPS: &[(&str, &str, &str)] = &[
    (
        "render --app",
        "An app's shot, rendered by its own adapter.",
        "APP_FLAGS",
    ),
    (
        "render --asset",
        "A logo, wordmark, lockup, tile or card on the path tracer.",
        "ASSET_FLAGS",
    ),
    (
        "render --icons",
        "An icon set on the path tracer.",
        "ICONS_FLAGS",
    ),
    (
        "render --scene",
        "A shot from a recipe folder's shots file on the path tracer.",
        "SCENE_FLAGS",
    ),
    ("list --shots", "An app's shots.", "SHOTS_FLAGS"),
    (
        "list --presets",
        "The recipe folder's presets.",
        "PRESETS_FLAGS",
    ),
    (
        "list --scenes",
        "The recipe folder's scene shots.",
        "SCENES_FLAGS",
    ),
    (
        "encode",
        "Re-encode a master from its manifest.",
        "ENCODE_FLAGS",
    ),
    ("cache", "The build cache.", "CACHE_FLAGS"),
    (
        "cache info",
        "The shared cache: its kinds, entries and size.",
        "CACHE_INFO_FLAGS",
    ),
    (
        "cache clean",
        "Remove shared cache entries by kind or age.",
        "CACHE_CLEAN_FLAGS",
    ),
    (
        "bake",
        "Path-traced probes, lightmaps, plates and detail maps.",
        "BAKE_FLAGS",
    ),
    (
        "shot",
        "A scene file rendered offline on the live renderer at a fixed clock.",
        "SHOT_FLAGS",
    ),
    (
        "steam stage",
        "Stage Steam's build scripts and depots.",
        "STEAM_FLAGS",
    ),
];
