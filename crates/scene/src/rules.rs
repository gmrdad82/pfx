use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::types::{
    Animation, BlendClip, Body, BodyShape, Camera, Casters, Character, CharacterKind, Color,
    Emitter, Finish, FinishKeys, Flare, Haze, Ink, KeyShape, KeyValue, Keys, LAYER_CAP,
    LAYER_COUNT, Light, LightKind, Lut, LutKey, Material, MotionBlur, Mover, MoverKind, Object,
    Particles, Physics, Plates, Preset, Projection, Proxy, ProxyKind, Rig, RigKind, Room,
    Scratches, Shadows, Sky, SkyKind, Sound, Sun, SunModel, Tape, Text, Tiles, Tone, ToneKind,
    Trace, Trigger, Tunable, TunableKind, TunableValue,
};

pub(crate) struct Issue {
    pub(crate) key: Vec<String>,
    pub(crate) message: String,
    pub(crate) code: Option<&'static str>,
}

#[derive(Default)]
pub(crate) struct Issues(pub(crate) Vec<Issue>);

impl Issues {
    fn at(&mut self, key: &[&str], message: impl Into<String>) {
        self.0.push(Issue {
            key: key.iter().map(|part| part.to_string()).collect(),
            message: message.into(),
            code: None,
        });
    }

    fn typed(&mut self, key: &[&str], message: impl Into<String>) {
        self.coded(key, crate::code::BAD_TYPE, message);
    }

    fn coded(&mut self, key: &[&str], code: &'static str, message: impl Into<String>) {
        self.0.push(Issue {
            key: key.iter().map(|part| part.to_string()).collect(),
            message: message.into(),
            code: Some(code),
        });
    }

    fn nested(&mut self, prefix: &[String], inner: Issues) {
        for mut issue in inner.0 {
            let mut key = prefix.to_vec();
            key.append(&mut issue.key);
            issue.key = key;
            self.0.push(issue);
        }
    }
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn nonnegative(values: &[f32]) -> bool {
    values
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
}

fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn name(kind: &str, name: &str) -> Option<String> {
    if name.trim().is_empty() || name.trim() != name {
        return Some(format!(
            "a {kind} name is empty or starts or ends with a space"
        ));
    }
    if kind == "material" && name.chars().any(char::is_control) {
        return Some(format!(
            "material name {name:?} needs printable characters without spaces at its ends"
        ));
    }
    None
}

pub(crate) fn object(object: &Object) -> Issues {
    let mut issues = Issues::default();
    let name = &object.name;
    if object.is_placement() {
        let taken: Vec<&str> = [
            ("mesh", object.mesh.is_some()),
            ("material", object.material.is_some()),
            ("materials", !object.materials.is_empty()),
            ("shadow", object.shadow.is_some()),
            ("two_sided", object.two_sided),
            ("clip", !object.clip.is_empty()),
            ("content", object.content.is_some()),
            ("pick", object.pick.is_some()),
            ("alpha_cutoff", object.alpha_cutoff.is_some()),
            ("face_camera", object.face_camera),
            ("body", object.body.is_some()),
            ("character", object.character.is_some()),
            ("trigger", object.trigger.is_some()),
            ("animation", object.animation.is_some()),
        ]
        .into_iter()
        .filter(|(_, present)| *present)
        .map(|(key, _)| key)
        .collect();
        for key in taken {
            issues.at(
                &[key],
                format!(
                    "object {name} places a prefab, which takes id, name, prefab, set, at, rotate, scale, parent, hidden, dynamic and authoring, not {key}"
                ),
            );
        }
    } else {
        if object.mesh.is_none() {
            issues.at(
                &[],
                format!("missing key 'mesh': object {name} names a mesh, or a prefab to place"),
            );
        }
        if !object.set.is_empty() {
            issues.at(
                &["set"],
                format!("object {name}: [object.set] belongs to an object that places a prefab"),
            );
        }
    }
    if !finite(&object.at) {
        issues.at(
            &["at"],
            format!("object {name} has a transform that is not finite"),
        );
    }
    if !finite(&object.rotate) {
        issues.at(
            &["rotate"],
            format!("object {name} has a transform that is not finite"),
        );
    }
    if !finite(&object.scale_axes()) {
        issues.at(
            &["scale"],
            format!("object {name} has a transform that is not finite"),
        );
    }
    if object.clip.len() > 2 {
        issues.at(
            &["clip"],
            format!(
                "object {name} has {} clip planes; at most two",
                object.clip.len()
            ),
        );
    } else if !finite(&object.clip.concat()) {
        issues.at(
            &["clip"],
            format!("object {name} has a clip plane that is not finite"),
        );
    }
    if let Some(cutoff) = object.alpha_cutoff
        && !(0.0..=1.0).contains(&cutoff)
    {
        issues.at(
            &["alpha_cutoff"],
            format!("object {name} alpha_cutoff = {cutoff} is outside 0 to 1"),
        );
    }
    if let Some(body) = &object.body {
        issues.nested(&["body".to_string()], self::body(name, body));
        if object.face_camera {
            issues.at(
                &["body"],
                format!("object {name} faces the camera; a card takes no body"),
            );
        }
    }
    if let Some(character) = &object.character {
        issues.nested(&["character".to_string()], self::character(name, character));
        if object.body.is_some() {
            issues.at(
                &["character"],
                format!("object {name}: a character or a body, not both"),
            );
        }
        if object.face_camera {
            issues.at(
                &["character"],
                format!("object {name} faces the camera; a card takes no character"),
            );
        }
        if object.dynamic == Some(false) {
            issues.at(
                &["dynamic"],
                format!("object {name} is a character, which moves; it takes no dynamic = false"),
            );
        }
    }
    if let Some(trigger) = &object.trigger {
        issues.nested(&["trigger".to_string()], self::trigger(name, trigger));
    }
    if let Some(animation) = &object.animation {
        issues.nested(&["animation".to_string()], self::animation(name, animation));
        if object.dynamic == Some(false) && object.character.is_none() {
            issues.at(
                &["dynamic"],
                format!("object {name} is animated, which changes it; it takes no dynamic = false"),
            );
        }
    }
    issues
}

struct Sized {
    shape: Option<BodyShape>,
    half: Option<[f32; 3]>,
    radius: Option<f32>,
    half_height: Option<f32>,
}

fn sizes(issues: &mut Issues, name: &str, what: &str, sized: Sized) {
    let shape = sized.shape.unwrap_or_default();
    let shape_name = match shape {
        BodyShape::Box => "box",
        BodyShape::Sphere => "sphere",
        BodyShape::Capsule => "capsule",
        BodyShape::Cylinder => "cylinder",
    };
    let foreign = match shape {
        BodyShape::Box => sized.radius.is_some() || sized.half_height.is_some(),
        BodyShape::Sphere => sized.half.is_some() || sized.half_height.is_some(),
        BodyShape::Capsule | BodyShape::Cylinder => sized.half.is_some(),
    };
    if foreign {
        issues.at(
            &["shape"],
            format!("object {name}: a {shape_name} {what} takes half for a box, radius for a sphere, radius and half_height for a capsule or cylinder"),
        );
    }
    let sizes: Vec<f32> = sized
        .half
        .into_iter()
        .flatten()
        .chain(sized.radius)
        .chain(sized.half_height)
        .collect();
    if sizes.iter().any(|size| !positive(*size)) {
        issues.at(
            &[],
            format!("object {name}: a {what}'s half, radius and half_height are positive"),
        );
    }
}

pub(crate) fn body(name: &str, body: &Body) -> Issues {
    let mut issues = Issues::default();
    let sized = Sized {
        shape: body.shape,
        half: body.half,
        radius: body.radius,
        half_height: body.half_height,
    };
    sizes(&mut issues, name, "body", sized);
    let fine = body.offset.is_none_or(|v| finite(&v))
        && body.velocity.is_none_or(|v| finite(&v))
        && body.spin.is_none_or(|v| finite(&v))
        && body.gravity_scale.is_none_or(f32::is_finite)
        && body.density.is_none_or(positive)
        && body.friction.is_none_or(|v| nonnegative(&[v]))
        && body.restitution.is_none_or(|v| (0.0..=1.0).contains(&v))
        && body.damping.is_none_or(|v| nonnegative(&v));
    if !fine {
        issues.at(
            &[],
            format!("object {name}: a body needs a positive density, a friction and damping of at least 0, a restitution of 0 to 1 and finite values"),
        );
    }
    issues
}

pub(crate) fn character(name: &str, character: &Character) -> Issues {
    let mut issues = Issues::default();
    if character.kind.unwrap_or_default() == CharacterKind::ThreeD {
        for (key, present) in [
            ("plane", character.plane.is_some()),
            ("variable_jump", character.variable_jump.is_some()),
            ("wall_slide", character.wall_slide.is_some()),
            ("wall_jump", character.wall_jump.is_some()),
        ] {
            if present {
                issues.at(
                    &[key],
                    format!("object {name}: {key} belongs to a 2d character, and this one is 3d"),
                );
            }
        }
    }
    for (key, value) in [("radius", character.radius), ("height", character.height)] {
        if let Some(value) = value
            && !positive(value)
        {
            issues.at(
                &[key],
                format!("object {name}: character {key} = {value} is not above 0"),
            );
        }
    }
    if let (Some(radius), Some(height)) = (character.radius, character.height)
        && positive(radius)
        && positive(height)
        && height < 2.0 * radius
    {
        issues.at(
            &["height"],
            format!("object {name}: character height {height} is less than twice its radius {radius}; the height counts both caps"),
        );
    }
    if let Some(climb) = character.max_climb
        && !(0.0..90.0).contains(&climb)
    {
        issues.at(
            &["max_climb"],
            format!("object {name}: character max_climb = {climb} is outside 0 to 90; a slope of 90 degrees is a wall"),
        );
    }
    for (key, value) in [
        ("max_step", character.max_step),
        ("snap", character.snap),
        ("jump_speed", character.jump_speed),
        ("wall_slide", character.wall_slide),
    ] {
        if let Some(value) = value
            && !nonnegative(&[value])
        {
            issues.at(
                &[key],
                format!("object {name}: character {key} = {value} is below 0 or not finite"),
            );
        }
    }
    if let (Some(step), Some(height)) = (character.max_step, character.height)
        && step >= height
    {
        issues.at(
            &["max_step"],
            format!("object {name}: character max_step {step} is not below its height {height}"),
        );
    }
    for (key, value) in [
        ("coyote", character.coyote),
        ("jump_buffer", character.jump_buffer),
    ] {
        if let Some(value) = value
            && value > Character::TICKS
        {
            issues.at(
                &[key],
                format!(
                    "object {name}: character {key} = {value} is outside 0 to {} ticks",
                    Character::TICKS
                ),
            );
        }
    }
    if let Some(cut) = character.variable_jump
        && !(0.0..=1.0).contains(&cut)
    {
        issues.at(
            &["variable_jump"],
            format!("object {name}: character variable_jump = {cut} is outside 0 to 1"),
        );
    }
    if let Some(jump) = character.wall_jump
        && !nonnegative(&jump)
    {
        issues.at(
            &["wall_jump"],
            format!("object {name}: character wall_jump is two speeds of at least 0, away and up"),
        );
    }
    issues
}

pub(crate) fn trigger(name: &str, trigger: &Trigger) -> Issues {
    let mut issues = Issues::default();
    let sized = Sized {
        shape: trigger.shape,
        half: trigger.half,
        radius: trigger.radius,
        half_height: trigger.half_height,
    };
    sizes(&mut issues, name, "trigger", sized);
    if !trigger.offset.is_none_or(|offset| finite(&offset)) {
        issues.at(
            &["offset"],
            format!("object {name}: a trigger's offset is not finite"),
        );
    }
    issues
}

fn clip_name(issues: &mut Issues, key: &[&str], object: &str, clip: &str) {
    if let Some(message) = name("clip", clip) {
        issues.coded(
            key,
            crate::code::BAD_NAME,
            format!("object {object}: {message}"),
        );
    }
}

pub(crate) fn animation(name: &str, animation: &Animation) -> Issues {
    let mut issues = Issues::default();
    let sprite = animation.is_sprite();
    if let Some(clip) = &animation.clip {
        clip_name(&mut issues, &["clip"], name, clip);
    }
    for (key, value) in [("speed", animation.speed), ("start", animation.start)] {
        if let Some(value) = value
            && !nonnegative(&[value])
        {
            issues.at(
                &[key],
                format!("object {name}: animation {key} = {value} is below 0 or not finite"),
            );
        }
    }
    if let Some(blend) = &animation.blend {
        if animation.clip.is_some() {
            issues.at(
                &["blend"],
                format!("object {name}: an animation plays clip or blend, not both"),
            );
        }
        if sprite {
            issues.at(
                &["blend"],
                format!("object {name}: a sprite shows one clip at a time; blend belongs to a glTF animation"),
            );
        }
        if blend.is_empty() {
            issues.at(&["blend"], format!("object {name}: blend lists no clip"));
        }
        let mut total = 0.0;
        for (place, entry) in blend.iter().enumerate() {
            let at = place.to_string();
            clip_name(&mut issues, &["blend", &at, "clip"], name, &entry.clip);
            if blend[..place].iter().any(|other| other.clip == entry.clip) {
                issues.at(
                    &["blend", &at, "clip"],
                    format!("object {name}: clip {} is in the blend twice", entry.clip),
                );
            }
            let weight = entry.weight.unwrap_or(BlendClip::WEIGHT);
            if nonnegative(&[weight]) {
                total += weight;
            } else {
                issues.at(
                    &["blend", &at, "weight"],
                    format!("object {name}: blend weight = {weight} is below 0 or not finite"),
                );
            }
        }
        if !blend.is_empty() && total <= 0.0 {
            issues.at(
                &["blend"],
                format!("object {name}: the blend's weights add up to 0; one at least is above 0"),
            );
        }
    }
    let count = animation.frame_count();
    if sprite {
        match (&animation.grid, &animation.frames) {
            (None, None) => issues.at(
                &[],
                format!("missing key 'grid': object {name}: a sprite cuts its atlas into frames by grid or frames"),
            ),
            (Some(_), Some(_)) => issues.at(
                &["frames"],
                format!("object {name}: a sprite cuts its atlas by grid or frames, not both"),
            ),
            _ => {}
        }
        if let Some([columns, rows]) = animation.grid
            && (columns == 0 || rows == 0)
        {
            issues.at(
                &["grid"],
                format!(
                    "object {name}: grid = [{columns}, {rows}] is columns and rows, each at least 1"
                ),
            );
        }
        if let Some(frames) = &animation.frames {
            if frames.is_empty() {
                issues.at(&["frames"], format!("object {name}: frames lists no frame"));
            }
            for (place, [_, _, width, height]) in frames.iter().enumerate() {
                if *width == 0 || *height == 0 {
                    issues.at(
                        &["frames", &place.to_string()],
                        format!("object {name}: frame {place} is [x, y, width, height] in pixels, with width and height at least 1"),
                    );
                }
            }
        }
        if let Some(fps) = animation.fps
            && !positive(fps)
        {
            issues.at(
                &["fps"],
                format!("object {name}: fps = {fps} is not above 0"),
            );
        }
        for (clip, range) in &animation.clips {
            clip_name(&mut issues, &["clips", clip], name, clip);
            if range.from > range.to {
                issues.at(
                    &["clips", clip, "to"],
                    format!(
                        "object {name}: clip {clip} runs from frame {} to {}; to is at least from",
                        range.from, range.to
                    ),
                );
            } else if let Some(count) = count
                && count > 0
                && range.to >= count
            {
                issues.at(
                    &["clips", clip, "to"],
                    format!(
                        "object {name}: clip {clip} ends at frame {}, past the atlas's last frame {}; frames count from 0",
                        range.to,
                        count - 1
                    ),
                );
            }
            if let Some(fps) = range.fps
                && !positive(fps)
            {
                issues.at(
                    &["clips", clip, "fps"],
                    format!("object {name}: clip {clip} fps = {fps} is not above 0"),
                );
            }
        }
        if let Some(clip) = &animation.clip
            && !animation.clips.contains_key(clip)
        {
            issues.coded(
                &["clip"],
                crate::code::BAD_REFERENCE,
                unknown_clip(name, clip, animation),
            );
        }
    } else {
        for (key, present) in [
            ("grid", animation.grid.is_some()),
            ("frames", animation.frames.is_some()),
            ("fps", animation.fps.is_some()),
            ("clips", !animation.clips.is_empty()),
        ] {
            if present {
                issues.at(
                    &[key],
                    format!("object {name}: {key} belongs to a sprite, which names its atlas; this animation plays the clips of the object's glTF"),
                );
            }
        }
    }
    for (place, event) in animation.events.iter().enumerate() {
        let place = place.to_string();
        let at = |key: &'static str| ["events", place.as_str(), key];
        let label = &event.name;
        if let Some(message) = self::name("event", label) {
            issues.coded(
                &at("name"),
                crate::code::BAD_NAME,
                format!("object {name}: {message}"),
            );
        }
        match (event.frame, event.time) {
            (None, None) => issues.at(
                &["events", &place],
                format!("missing key 'time': object {name}: event {label} takes a time, or a sprite's frame"),
            ),
            (Some(_), Some(_)) => issues.at(
                &at("frame"),
                format!("object {name}: event {label} takes time or frame, not both"),
            ),
            _ => {}
        }
        if let Some(time) = event.time
            && !nonnegative(&[time])
        {
            issues.at(
                &at("time"),
                format!("object {name}: event {label} time = {time} is below 0 or not finite"),
            );
        }
        if let Some(clip) = &event.clip {
            clip_name(&mut issues, &at("clip"), name, clip);
        }
        let length = if sprite && animation.clips.is_empty() {
            if let Some(clip) = &event.clip {
                issues.coded(
                    &at("clip"),
                    crate::code::BAD_REFERENCE,
                    unknown_clip(name, clip, animation),
                );
            }
            count
        } else {
            match event.clip.as_ref().or(animation.clip.as_ref()) {
                None => {
                    issues.at(
                        &["events", &place],
                        format!("missing key 'clip': object {name}: event {label} names its clip, since the animation plays none from the start"),
                    );
                    None
                }
                Some(clip) if sprite => match animation.clips.get(clip) {
                    Some(range) => Some(range.count()),
                    None => {
                        if event.clip.is_some() {
                            issues.coded(
                                &at("clip"),
                                crate::code::BAD_REFERENCE,
                                unknown_clip(name, clip, animation),
                            );
                        }
                        None
                    }
                },
                Some(_) => None,
            }
        };
        if let Some(frame) = event.frame {
            if !sprite {
                issues.at(
                    &at("frame"),
                    format!("object {name}: a glTF clip has no frames; event {label} takes a time"),
                );
            } else if let Some(length) = length
                && length > 0
                && frame >= length
            {
                issues.at(
                    &at("frame"),
                    format!(
                        "object {name}: event {label} is at frame {frame}, past its clip's last frame {}; frames count from 0 within the clip",
                        length - 1
                    ),
                );
            }
        }
    }
    issues
}

fn unknown_clip(name: &str, clip: &str, animation: &Animation) -> String {
    if animation.clips.is_empty() {
        format!(
            "object {name}: clip {clip} names no clip; this sprite has no clips, and plays every frame of its atlas"
        )
    } else {
        format!(
            "object {name}: clip {clip} names no clip of clips, which names {}",
            animation
                .clips
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

pub(crate) fn palette_key(key: &str) -> Option<String> {
    if key.len() == 1 && key.starts_with(Tiles::EMPTY) {
        return Some("'.' marks an empty cell and names no tile".to_string());
    }
    if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some(format!(
            "palette key {key:?} is empty or holds a space or a control character"
        ));
    }
    None
}

pub(crate) fn tiles(tiles: &Tiles) -> Issues {
    let mut issues = Issues::default();
    let name = &tiles.name;
    if !tiles.cell_sides().iter().all(|side| positive(*side)) {
        issues.at(
            &["cell"],
            format!("tile layer {name}: a cell's sides are above 0"),
        );
    }
    if !finite(&tiles.origin) {
        issues.at(
            &["origin"],
            format!("tile layer {name}: origin is not finite"),
        );
    }
    if tiles.rows.is_some() {
        for key in tiles.palette.keys() {
            if palette_key(key).is_none() && key.chars().count() != 1 {
                issues.at(
                    &["palette", key],
                    format!("tile layer {name}: palette key {key} is longer than one character, and rows name each tile by one character"),
                );
            }
        }
    }
    if tiles.rows.is_some() && tiles.cells.is_some() {
        issues.at(
            &["cells"],
            format!("tile layer {name}: its cells are rows or a list of cells, not both"),
        );
    }
    let mut seen: BTreeMap<[i32; 2], usize> = BTreeMap::new();
    for (place, cell) in tiles.cells.iter().flatten().enumerate() {
        let at = place.to_string();
        if !tiles.palette.contains_key(&cell.tile) {
            issues.coded(
                &["cells", &at, "tile"],
                crate::code::BAD_REFERENCE,
                unknown_tile(name, &cell.tile, tiles),
            );
        }
        match seen.get(&cell.at) {
            Some(first) => issues.at(
                &["cells", &at, "at"],
                format!(
                    "tile layer {name}: cell [{}, {}] is set twice; cell {first} sets it already",
                    cell.at[0], cell.at[1]
                ),
            ),
            None => {
                seen.insert(cell.at, place);
            }
        }
    }
    issues
}

pub(crate) fn unknown_tile(name: &str, tile: &str, tiles: &Tiles) -> String {
    if tiles.palette.is_empty() {
        format!("tile layer {name}: tile {tile} names no tile; the layer's palette is empty")
    } else {
        format!(
            "tile layer {name}: tile {tile} names no tile of its palette, which names {}",
            tiles.palette.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    }
}

pub(crate) fn rig(rig: &Rig) -> Issues {
    let mut issues = Issues::default();
    let name = &rig.name;
    let kind = rig.kind.name();
    let follow = [
        ("dead_zone", rig.dead_zone.is_some()),
        ("look_ahead", rig.look_ahead.is_some()),
        ("damping", rig.damping.is_some()),
        ("bounds", rig.bounds.is_some()),
        ("orthographic", rig.orthographic.is_some()),
        ("snap", rig.snap.is_some()),
    ];
    let look = [
        ("sensitivity", rig.sensitivity.is_some()),
        ("invert", rig.invert.is_some()),
        ("smoothing", rig.smoothing.is_some()),
        ("pitch", rig.pitch.is_some()),
    ];
    let bob = [("head_bob", rig.head_bob.is_some())];
    let orbit = [
        ("distance", rig.distance.is_some()),
        ("collide", rig.collide.is_some()),
    ];
    let (takes, foreign) = match rig.kind {
        RigKind::Follow => (
            "target, offset, dead_zone, look_ahead, damping, bounds, orthographic and snap",
            [&look[..], &bob, &orbit].concat(),
        ),
        RigKind::FirstPerson => (
            "target, offset, sensitivity, invert, smoothing, pitch and head_bob",
            [&follow[..], &orbit].concat(),
        ),
        RigKind::ThirdPerson => (
            "target, offset, sensitivity, invert, smoothing, pitch, distance and collide",
            [&follow[..], &bob].concat(),
        ),
    };
    for (key, present) in foreign {
        if present {
            issues.at(
                &[key],
                format!("rig {name}: a {kind} rig takes {takes}, not {key}"),
            );
        }
    }
    if !rig.offset.is_none_or(|offset| finite(&offset)) {
        issues.at(&["offset"], format!("rig {name}: offset is not finite"));
    }
    let measures = [
        ("dead_zone", rig.dead_zone.map(Vec::from)),
        ("look_ahead", rig.look_ahead.map(|value| vec![value])),
        ("damping", rig.damping.map(Vec::from)),
        ("smoothing", rig.smoothing.map(|value| vec![value])),
        ("head_bob", rig.head_bob.map(|value| vec![value])),
    ];
    for (key, values) in measures {
        if let Some(values) = values
            && !nonnegative(&values)
        {
            issues.at(
                &[key],
                format!("rig {name}: {key} is at least 0 and finite"),
            );
        }
    }
    for (key, value) in [
        ("orthographic", rig.orthographic),
        ("snap", rig.snap),
        ("sensitivity", rig.sensitivity),
        ("distance", rig.distance),
    ] {
        if let Some(value) = value
            && !positive(value)
        {
            issues.at(
                &[key],
                format!("rig {name}: {key} = {value} is not above 0"),
            );
        }
    }
    if rig.kind == RigKind::Follow && rig.snap.is_some() && rig.orthographic.is_none() {
        issues.at(
            &["snap"],
            format!("rig {name}: snap moves an orthographic view by whole pixels, and needs orthographic"),
        );
    }
    if let Some(bounds) = rig.bounds {
        if !finite(&bounds.min) || !finite(&bounds.max) {
            issues.at(&["bounds"], format!("rig {name}: bounds are not finite"));
        } else if let Some(axis) = (0..3).find(|axis| bounds.min[*axis] > bounds.max[*axis]) {
            issues.at(
                &["bounds"],
                format!(
                    "rig {name}: bounds min is above max in {}",
                    ["x", "y", "z"][axis]
                ),
            );
        }
    }
    if let Some([low, high]) = rig.pitch
        && !((-90.0..=90.0).contains(&low) && (-90.0..=90.0).contains(&high) && low <= high)
    {
        issues.at(
            &["pitch"],
            format!("rig {name}: pitch = [{low}, {high}]; it is [min, max] in degrees, each from -90 to 90, min at most max"),
        );
    }
    issues
}

pub(crate) fn layers(layers: &BTreeMap<String, Vec<String>>) -> Issues {
    let mut issues = Issues::default();
    if let Some(past) = layers.keys().nth(LAYER_COUNT) {
        issues.at(
            &[past],
            format!(
                "the project names {} layers; at most {LAYER_COUNT}",
                layers.len()
            ),
        );
    }
    for (name, row) in layers {
        for (place, other) in row.iter().enumerate() {
            let at = [name.as_str(), &place.to_string()];
            if row[..place].contains(other) {
                issues.at(&at, format!("layer {name} lists {other} twice"));
            } else if let Some(back) = layers.get(other) {
                if !back.contains(name) {
                    issues.at(
                        &at,
                        format!("layer {name} meets {other}, and {other} does not list {name}; each pair is listed on both sides"),
                    );
                }
            } else {
                issues.coded(
                    &at,
                    crate::code::BAD_REFERENCE,
                    format!("{other} names no layer of [layers]"),
                );
            }
        }
    }
    issues
}

pub(crate) fn layer_names(object: &Object) -> Vec<(Vec<String>, String)> {
    let mut names = Vec::new();
    let mut add = |path: &[&str], name: &Option<String>| {
        if let Some(name) = name {
            names.push((
                path.iter().map(|part| part.to_string()).collect(),
                name.clone(),
            ));
        }
    };
    if let Some(body) = &object.body {
        add(&["body", "layer"], &body.layer);
    }
    if let Some(character) = &object.character {
        add(&["character", "layer"], &character.layer);
    }
    if let Some(trigger) = &object.trigger {
        add(&["trigger", "layer"], &trigger.layer);
        for (place, name) in trigger.mask.iter().flatten().enumerate() {
            add(
                &["trigger", "mask", &place.to_string()],
                &Some(name.clone()),
            );
        }
    }
    names
}

pub(crate) fn unknown_layer(layers: &BTreeMap<String, Vec<String>>, name: &str) -> Option<String> {
    if layers.contains_key(name) {
        return None;
    }
    Some(if layers.is_empty() {
        format!("layer {name}: the project names no layers; [layers] in project.toml names them")
    } else {
        format!(
            "{name} names no layer of the project; [layers] in project.toml names {}",
            layers.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    })
}

fn unit(issues: &mut Issues, key: &[&str], value: Option<f32>) {
    if let Some(value) = value
        && !(0.0..=1.0).contains(&value)
    {
        issues.at(
            key,
            format!("{} = {value} is outside 0 to 1", key.join(".")),
        );
    }
}

fn unit_color(issues: &mut Issues, key: &[&str], value: Option<[f32; 3]>) {
    if let Some(value) = value
        && !value.iter().all(|part| (0.0..=1.0).contains(part))
    {
        issues.at(
            key,
            format!("{} = {value:?} is outside 0 to 1", key.join(".")),
        );
    }
}

fn at_least(issues: &mut Issues, key: &str, value: Option<f32>, low: f32) {
    if let Some(value) = value
        && !(value.is_finite() && value >= low)
    {
        issues.at(
            &[key],
            format!("{key} = {value} is a number of at least {low}"),
        );
    }
}

fn span(issues: &mut Issues, key: &str, value: Option<[f32; 2]>) {
    if let Some([low, high]) = value
        && !(nonnegative(&[low, high]) && low <= high)
    {
        issues.at(
            &[key],
            format!("{key} is [low, high], both at least 0 and low at most high"),
        );
    }
}

fn heading(issues: &mut Issues, key: &str, value: Option<[f32; 3]>) {
    if let Some(value) = value
        && !(finite(&value) && dot(value, value) > 1e-12)
    {
        issues.at(&[key], format!("{key} is a finite nonzero vector"));
    }
}

fn node(
    issues: &mut Issues,
    kind: &str,
    name: &str,
    parent: &Option<String>,
    node: &Option<String>,
) {
    match (parent, node) {
        (_, Some(node)) if node.trim().is_empty() => {
            issues.at(&["node"], format!("{kind} {name}: node names a glTF node"));
        }
        (None, Some(_)) => issues.at(
            &["node"],
            format!("{kind} {name}: node names a glTF node inside its parent and needs a parent"),
        ),
        _ => {}
    }
}

fn unit_of(value: f32) -> Option<&'static str> {
    (!(0.0..=1.0).contains(&value)).then_some("are 0 to 1")
}

fn at_least_zero(value: f32) -> Option<&'static str> {
    (!nonnegative(&[value])).then_some("are at least 0")
}

fn above_zero(value: f32) -> Option<&'static str> {
    (!positive(value)).then_some("are above 0")
}

fn shown(value: &KeyValue) -> String {
    match value {
        KeyValue::Number(value) => value.to_string(),
        KeyValue::Pair(value) => format!("{value:?}"),
        KeyValue::Triple(value) => format!("{value:?}"),
    }
}

fn keys(
    issues: &mut Issues,
    what: &str,
    keys: &Option<Keys>,
    keyed: &[(&str, KeyShape)],
    check: impl Fn(&str, &KeyValue) -> Option<&'static str>,
) {
    let Some(keys) = keys else { return };
    let names = keyed
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    for (name, track) in &keys.0 {
        let Some((_, shape)) = keyed.iter().find(|(known, _)| known == name) else {
            issues.at(
                &["keys", name],
                format!("{what}: keys.{name} names no keyable key; it keys {names}"),
            );
            continue;
        };
        if track.is_empty() {
            issues.at(
                &["keys", name],
                format!("{what}: keys.{name} holds [seconds, value] keys, at least one"),
            );
        }
        let mut last: Option<f32> = None;
        for (place, (seconds, value)) in track.iter().enumerate() {
            let place = place.to_string();
            if !(seconds.is_finite() && *seconds >= 0.0) {
                issues.at(
                    &["keys", name, &place, "0"],
                    format!("{what}: keys.{name} times are seconds of at least 0, not {seconds}"),
                );
            } else if let Some(before) = last
                && *seconds <= before
            {
                issues.at(
                    &["keys", name, &place, "0"],
                    format!("{what}: keys.{name} times rise strictly; {seconds} follows {before}"),
                );
            }
            last = Some(*seconds);
            if value.shape() != *shape {
                issues.at(
                    &["keys", name, &place, "1"],
                    format!(
                        "{what}: keys.{name} values are {}, not {}",
                        shape.describe(),
                        shown(value)
                    ),
                );
            } else if !finite(value.parts()) {
                issues.at(
                    &["keys", name, &place, "1"],
                    format!(
                        "{what}: keys.{name} values are finite numbers, not {}",
                        shown(value)
                    ),
                );
            } else if let Some(wrong) = check(name, value) {
                issues.at(
                    &["keys", name, &place, "1"],
                    format!("{what}: keys.{name} values {wrong}, not {}", shown(value)),
                );
            }
        }
    }
}

fn keyed(keys: &Option<Keys>, name: &str) -> bool {
    keys.as_ref().is_some_and(|keys| keys.has(name))
}

fn cone_of([inner, outer]: [f32; 2]) -> Option<&'static str> {
    (!(0.0 <= inner && inner <= outer && outer <= 180.0))
        .then_some("are [inner, outer] degrees, 0 <= inner <= outer <= 180")
}

fn span_of([low, high]: [f32; 2]) -> Option<&'static str> {
    (!(nonnegative(&[low, high]) && low <= high))
        .then_some("are [low, high], both at least 0 and low at most high")
}

fn light_key(name: &str, value: &KeyValue) -> Option<&'static str> {
    match (name, value) {
        ("intensity", KeyValue::Number(value)) => at_least_zero(*value),
        ("range", KeyValue::Number(value)) => above_zero(*value),
        ("color", KeyValue::Triple(value)) => {
            (!nonnegative(value)).then_some("are three numbers of at least 0")
        }
        ("cone", KeyValue::Pair(value)) => cone_of(*value),
        _ => None,
    }
}

pub(crate) fn light(light: &Light) -> Issues {
    let mut issues = Issues::default();
    let name = &light.name;
    if light.kind.unwrap_or_default() == LightKind::Point {
        for (key, present) in [
            ("direction", light.direction.is_some()),
            ("cone", light.cone.is_some() || keyed(&light.keys, "cone")),
        ] {
            if present {
                issues.at(
                    &[key],
                    format!("light {name}: {key} belongs to a spot light (kind = \"spot\")"),
                );
            }
        }
    }
    heading(&mut issues, "direction", light.direction);
    if let Some([inner, outer]) = light.cone
        && !(inner.is_finite()
            && outer.is_finite()
            && 0.0 <= inner
            && inner <= outer
            && outer <= 180.0)
    {
        issues.at(
            &["cone"],
            format!("light {name}: cone is [inner, outer] degrees, 0 <= inner <= outer <= 180"),
        );
    }
    node(&mut issues, "light", name, &light.parent, &light.node);
    keys(
        &mut issues,
        &format!("light {name}"),
        &light.keys,
        Light::KEYED,
        light_key,
    );
    if light.parent.is_none() && light.at.is_some() {
        issues.at(
            &["at"],
            format!("light {name}: at places a light in its parent's frame and needs a parent"),
        );
    }
    if !finite(&light.at.unwrap_or([0.0; 3])) {
        issues.at(&["at"], format!("light {name}: at is three finite numbers"));
    }
    let color = light.color.unwrap_or(Light::COLOR);
    let fine = finite(&light.position)
        && nonnegative(&color)
        && nonnegative(&[light.intensity, light.radius.unwrap_or(Light::RADIUS)])
        && positive(light.range.unwrap_or(Light::RANGE));
    if !fine {
        issues.at(
            &[],
            format!(
                "light {} needs a finite position, a nonnegative color, intensity and radius, and a positive range",
                light.name
            ),
        );
    }
    issues
}

pub(crate) fn emitter(emitter: &Emitter) -> Issues {
    let mut issues = Issues::default();
    let fine = finite(&emitter.position)
        && nonnegative(&emitter.color.unwrap_or([1.0; 3]))
        && nonnegative(&[emitter.intensity])
        && positive(emitter.radius);
    if !fine {
        issues.at(
            &[],
            format!(
                "emitter {} needs a finite position, a positive radius and a nonnegative color and intensity",
                emitter.name
            ),
        );
    }
    issues
}

pub(crate) fn mover(mover: &Mover) -> Issues {
    let mut issues = Issues::default();
    let name = &mover.name;
    if mover.kind == MoverKind::Slide && mover.pivot.is_some() {
        issues.at(&["pivot"], format!("mover {name}: a slide takes no pivot"));
    }
    if mover.objects.is_empty() {
        issues.at(&["objects"], format!("mover {name} moves no objects"));
    }
    let pivot = mover.pivot.unwrap_or([0.0; 3]);
    let fine = finite(&pivot)
        && finite(&mover.axis)
        && dot(mover.axis, mover.axis) > 1e-12
        && finite(&mover.travel)
        && positive(mover.period.unwrap_or(4.0))
        && mover.offset.unwrap_or(0.0).is_finite();
    if !fine {
        issues.at(
            &[],
            format!("mover {name} needs a finite pivot, a nonzero axis, a finite travel and offset, and a positive period"),
        );
    }
    if mover.clip.len() > 2 || !finite(&mover.clip.concat()) {
        issues.at(
            &["clip"],
            format!("mover {name} needs at most two finite clip planes"),
        );
    }
    issues
}

pub(crate) fn text(text: &Text) -> Issues {
    let mut issues = Issues::default();
    if !positive(text.size)
        || !nonnegative(&text.color.unwrap_or(Text::COLOR))
        || !finite(&text.at)
        || !finite(&text.rotate)
    {
        issues.at(
            &[],
            "text needs a positive size, a nonnegative color and a finite placement",
        );
    }
    issues
}

pub(crate) fn sound(name: &str, sound: &Sound) -> Issues {
    use crate::types::Cue;
    let mut issues = Issues::default();
    match (sound.play.unwrap_or_default(), &sound.object) {
        (Cue::Hit, None) => issues.at(
            &["play"],
            format!("sound {name} plays on a hit and names no object"),
        ),
        (Cue::Start | Cue::Game, Some(_)) => issues.at(
            &["object"],
            format!("sound {name}: object belongs to a sound played on a hit"),
        ),
        _ => {}
    }
    if !nonnegative(&[sound.volume.unwrap_or(1.0)])
        || !(-1.0..=1.0).contains(&sound.pan.unwrap_or(0.0))
    {
        issues.at(
            &[],
            format!("sound {name} needs a volume of at least 0 and a pan of -1 to 1"),
        );
    }
    issues
}

pub(crate) fn particles(particles: &Particles) -> Issues {
    let mut issues = Issues::default();
    let name = &particles.name;
    node(
        &mut issues,
        "particle system",
        name,
        &particles.parent,
        &particles.node,
    );
    if !finite(&particles.at.unwrap_or([0.0; 3])) {
        issues.at(
            &["at"],
            format!("particle system {name}: at is three finite numbers"),
        );
    }
    at_least(&mut issues, "rate", particles.rate, 0.0);
    if let Some(keys) = &particles.rate_keys {
        let fine = keys.iter().all(|[t, rate]| nonnegative(&[*t, *rate]))
            && keys.windows(2).all(|pair| pair[0][0] <= pair[1][0]);
        if !fine {
            issues.at(
                &["rate_keys"],
                format!("particle system {name}: rate_keys are [seconds, rate] pairs, both at least 0, in time order"),
            );
        }
    }
    if particles.rate_keys.is_some() && keyed(&particles.keys, "rate") {
        issues.at(
            &["rate_keys"],
            format!("particle system {name}: rate_keys and keys.rate are one track; write one"),
        );
    }
    keys(
        &mut issues,
        &format!("particle system {name}"),
        &particles.keys,
        Particles::KEYED,
        |key, value| match (key, value) {
            ("rate", KeyValue::Number(value)) => at_least_zero(*value),
            ("alpha", KeyValue::Number(value)) => unit_of(*value),
            ("speed", KeyValue::Pair(value)) => span_of(*value),
            ("size", KeyValue::Pair(value)) => {
                (!nonnegative(value)).then_some("are two numbers of at least 0")
            }
            _ => None,
        },
    );
    if particles.max == Some(0) {
        issues.at(
            &["max"],
            format!("particle system {name}: max is at least 1"),
        );
    }
    span(&mut issues, "lifetime", particles.lifetime);
    if let Some([low, _]) = particles.lifetime
        && low <= 0.0
    {
        issues.at(
            &["lifetime"],
            format!("particle system {name}: lifetime is above 0"),
        );
    }
    span(&mut issues, "speed", particles.speed);
    heading(&mut issues, "direction", particles.direction);
    if let Some(cone) = particles.cone
        && !(0.0..=180.0).contains(&cone)
    {
        issues.at(
            &["cone"],
            format!("particle system {name}: cone = {cone} is outside 0 to 180 degrees"),
        );
    }
    unit(&mut issues, &["inherit"], particles.inherit);
    if !finite(&particles.gravity.unwrap_or([0.0; 3])) {
        issues.at(
            &["gravity"],
            format!("particle system {name}: gravity is three finite numbers"),
        );
    }
    at_least(&mut issues, "drag", particles.drag, 0.0);
    if let Some(size) = particles.size
        && !nonnegative(&size)
    {
        issues.at(
            &["size"],
            format!("particle system {name}: size is two numbers of at least 0"),
        );
    }
    if let Some(alpha) = &particles.alpha
        && !alpha
            .iter()
            .all(|pair| pair.iter().all(|value| (0.0..=1.0).contains(value)))
    {
        issues.at(
            &["alpha"],
            format!("particle system {name}: alpha is [t, alpha] pairs, both 0 to 1"),
        );
    }
    color(&mut issues, "color", &particles.color);
    if let Some(colors) = &particles.colors
        && !colors
            .iter()
            .all(|(t, color)| (0.0..=1.0).contains(t) && color.valid())
    {
        issues.at(
            &["colors"],
            format!("particle system {name}: colors are [t, \"#rrggbb\"] pairs, t 0 to 1"),
        );
    }
    if let Some(spin) = particles.spin
        && !finite(&spin)
    {
        issues.at(
            &["spin"],
            format!("particle system {name}: spin is two finite numbers"),
        );
    }
    at_least(&mut issues, "stretch", particles.stretch, 0.0);
    issues
}

pub(crate) fn flare(flare: &Flare) -> Issues {
    let mut issues = Issues::default();
    unit(&mut issues, &["amount"], flare.amount);
    for (key, value) in [
        ("ghost_spacing", flare.ghost_spacing),
        ("halo", flare.halo),
        ("halo_radius", flare.halo_radius),
        ("starburst", flare.starburst),
        ("threshold", flare.threshold),
    ] {
        at_least(&mut issues, key, value, 0.0);
    }
    issues
}

pub(crate) fn motion_blur(motion_blur: &MotionBlur) -> Issues {
    let mut issues = Issues::default();
    unit(&mut issues, &["shutter"], motion_blur.shutter);
    at_least(&mut issues, "max_px", motion_blur.max_px, 0.0);
    if motion_blur.samples == Some(0) {
        issues.at(&["samples"], "motion_blur samples is at least 1");
    }
    issues
}

pub(crate) fn haze(haze: &Haze) -> Issues {
    let mut issues = Issues::default();
    match (haze.lo, haze.hi) {
        (Some(lo), Some(hi)) => {
            if !finite(&lo) || !finite(&hi) || (0..3).any(|k| lo[k] >= hi[k]) {
                issues.at(&[], "haze lo must lie below hi on every axis");
            }
        }
        (None, None) if haze.outdoor() => {}
        (lo, _) => {
            let key = if lo.is_none() { "lo" } else { "hi" };
            issues.at(
                &[key],
                format!("missing key '{key}': a room haze needs lo and hi; an outdoor haze sets height or falloff"),
            );
        }
    }
    if let Some(height) = haze.height
        && !height.is_finite()
    {
        issues.at(&["height"], "haze height is a finite number");
    }
    at_least(&mut issues, "falloff", haze.falloff, 0.0);
    unit(&mut issues, &["shafts"], haze.shafts);
    if haze.steps == Some(0) {
        issues.at(&["steps"], "haze steps is at least 1");
    }
    color(&mut issues, "color", &haze.color);
    keys(
        &mut issues,
        "haze",
        &haze.keys,
        Haze::KEYED,
        |key, value| match (key, value) {
            ("amount", KeyValue::Number(value)) => unit_of(*value),
            _ => None,
        },
    );
    let amount = haze.amount.unwrap_or(0.0);
    if !(0.0..=1.0).contains(&amount) {
        issues.at(
            &["amount"],
            format!("haze amount = {amount} is outside 0 to 1"),
        );
    }
    let values: Vec<f32> = [
        haze.fog,
        haze.smoke,
        haze.mist,
        haze.floor,
        haze.back,
        haze.reach,
        haze.unmapped,
    ]
    .into_iter()
    .flatten()
    .chain(haze.gold.into_iter().flatten())
    .chain(haze.ambient.into_iter().flatten())
    .collect();
    if !nonnegative(&values) {
        issues.at(
            &[],
            "haze fog, smoke, mist, floor, back, reach, unmapped, gold and ambient must be nonnegative",
        );
    }
    let phase = haze.phase.unwrap_or(0.3);
    if !(-0.9..=0.9).contains(&phase) {
        issues.at(
            &["phase"],
            format!("haze phase = {phase} is outside -0.9 to 0.9"),
        );
    }
    issues
}

fn daylight(issues: &mut Issues, sun: &Sun, hour: Option<f64>) {
    let fields = [
        ("hour", hour, 0.0, 24.0),
        ("day", sun.day, 1.0, 365.0),
        ("latitude", sun.latitude, -90.0, 90.0),
        ("heading", sun.heading, 0.0, 360.0),
    ];
    for (key, value, low, high) in fields {
        match value {
            Some(value) if !(low..=high).contains(&value) => issues.at(
                &[key],
                format!("daylight.{key} = {value} is outside {low} to {high}"),
            ),
            None if key == "hour" => issues.at(&[], "daylight needs an hour, 0 to 24"),
            _ => {}
        }
    }
}

pub(crate) fn sun(sun: &Sun) -> Issues {
    let mut issues = Issues::default();
    let radius = sun.radius.unwrap_or(0.0);
    if !(0.0..=10.0).contains(&radius) {
        issues.at(
            &["radius"],
            format!("sun radius = {radius} is outside 0 to 10"),
        );
    }
    keys(
        &mut issues,
        "sun",
        &sun.keys,
        Sun::KEYED,
        |key, value| match (key, value) {
            ("hour", KeyValue::Number(value)) => {
                (!(0.0..=24.0).contains(value)).then_some("are 0 to 24")
            }
            _ => None,
        },
    );
    let reference = sun.reference_hour.unwrap_or(Sun::REFERENCE_HOUR);
    if !(0.0..=24.0).contains(&reference) {
        issues.at(
            &["reference_hour"],
            format!("reference_hour = {reference} is outside 0 to 24"),
        );
    }
    match sun.model {
        SunModel::Daylight => {
            if sun.toward.is_some() || sun.color.is_some() || sun.irradiance.is_some() {
                issues.at(&["model"], "a daylight sun takes hour, day, latitude, heading, reference_hour and radius, not toward, color or irradiance");
            }
            daylight(&mut issues, sun, sun.hour);
        }
        SunModel::Authored => {
            match (sun.toward, sun.irradiance) {
                (Some(toward), Some(irradiance)) => {
                    let color = sun.color.unwrap_or([1.0; 3]);
                    if !finite(&toward)
                        || dot(toward, toward) <= 0.0
                        || !nonnegative(&color)
                        || !nonnegative(&[irradiance])
                    {
                        issues.at(&[], "an authored sun needs a nonzero toward and a nonnegative color and irradiance");
                    }
                }
                _ => issues.at(&[], "an authored sun needs toward and irradiance"),
            }
            daylight(&mut issues, sun, Some(sun.hour.unwrap_or(reference)));
        }
    }
    issues
}

fn room(issues: &mut Issues, room: &Room) {
    if !(2..=Room::MAX_WIDTH).contains(&room.width) {
        issues.at(
            &["room", "width"],
            format!(
                "room width is {}, it must be 2 to {}",
                room.width,
                Room::MAX_WIDTH
            ),
        );
    }
    if !nonnegative(&room.floor) || !nonnegative(&room.wall) || !nonnegative(&room.ceiling) {
        issues.at(
            &["room"],
            "room floor, wall and ceiling must be finite and nonnegative",
        );
    }
    for (place, lamp) in room.lights.iter().enumerate() {
        let fine = nonnegative(&lamp.color)
            && finite(&[lamp.az, lamp.el, lamp.open])
            && nonnegative(&[lamp.power, lamp.soft, lamp.slats])
            && [lamp.width, lamp.height]
                .iter()
                .all(|v| v.is_finite() && *v > 0.0 && *v < 180.0);
        if !fine {
            issues.at(
                &["room", "lights", &place.to_string()],
                format!("room lamp {:?} needs finite values, nonnegative power, colour, soft and slats, and a width and height between 0 and 180 degrees", lamp.name),
            );
        }
    }
}

fn one_sky(issues: &mut Issues, sky: &Sky) {
    let rotation = sky.rotation_deg.unwrap_or(0.0);
    let intensity = sky.intensity.unwrap_or(1.0);
    if !rotation.is_finite() || !nonnegative(&[intensity]) {
        issues.at(
            &[],
            "sky rotation_deg must be finite and intensity nonnegative",
        );
    }
    match sky.kind {
        SkyKind::Analytic => {
            if sky.path.is_some()
                || sky.prepare.is_some()
                || sky.room.is_some()
                || sky.rotation_deg.is_some()
            {
                issues.at(&["kind"], "an analytic sky takes turbidity, ground_albedo, ambient and intensity; it follows the sun");
            }
            let turbidity = sky.turbidity.unwrap_or(Sky::TURBIDITY);
            if !(1.0..=10.0).contains(&turbidity)
                || !nonnegative(&sky.ground_albedo.unwrap_or(Sky::GROUND_ALBEDO))
                || sky.ambient.is_some_and(|a| !nonnegative(&[a]))
            {
                issues.at(&[], "an analytic sky needs turbidity 1 to 10, a nonnegative ground_albedo and ambient");
            }
        }
        SkyKind::Hdr => {
            if sky.room.is_some()
                || sky.turbidity.is_some()
                || sky.ground_albedo.is_some()
                || sky.ambient.is_some()
            {
                issues.at(
                    &["kind"],
                    "an hdr sky takes path, rotation_deg, intensity and [sky.prepare]",
                );
            }
            if sky.path.is_none() {
                issues.at(&[], "an hdr sky needs a path");
            }
            if let Some(prepare) = sky.prepare {
                if prepare.cap.is_none() && prepare.balance.is_none() && prepare.mean.is_none() {
                    issues.at(&["prepare"], "[sky.prepare] needs cap, balance or mean");
                }
                if !prepare.cap.is_none_or(positive)
                    || !prepare.mean.is_none_or(positive)
                    || !prepare.balance.is_none_or(|b| b.into_iter().all(positive))
                {
                    issues.at(
                        &["prepare"],
                        "[sky.prepare] cap, balance and mean must be positive",
                    );
                }
            }
        }
        SkyKind::Room => {
            if sky.path.is_some()
                || sky.prepare.is_some()
                || sky.turbidity.is_some()
                || sky.ground_albedo.is_some()
                || sky.ambient.is_some()
            {
                issues.at(
                    &["kind"],
                    "a room sky takes [sky.room], rotation_deg and intensity",
                );
            }
            match &sky.room {
                Some(table) => room(issues, table),
                None => issues.at(&[], "a room sky needs a [sky.room] table"),
            }
        }
        SkyKind::Mix => {}
    }
}

fn sky_key(name: &str, value: &KeyValue) -> Option<&'static str> {
    match (name, value) {
        ("intensity", KeyValue::Number(value)) => at_least_zero(*value),
        ("backdrop", KeyValue::Number(value)) => above_zero(*value),
        ("horizon_amount", KeyValue::Number(value)) => unit_of(*value),
        ("horizon_width", KeyValue::Number(value)) => {
            (!(*value > 0.0 && *value <= 90.0)).then_some("are degrees above 0, at most 90")
        }
        ("horizon_color", KeyValue::Triple(value)) => {
            (!nonnegative(value)).then_some("are three numbers of at least 0")
        }
        _ => None,
    }
}

fn seen(sky: &Sky) -> [(&'static str, Option<KeyValue>); 4] {
    [
        ("backdrop", sky.backdrop.map(KeyValue::Number)),
        ("horizon_color", sky.horizon_color.map(KeyValue::Triple)),
        ("horizon_width", sky.horizon_width.map(KeyValue::Number)),
        ("horizon_amount", sky.horizon_amount.map(KeyValue::Number)),
    ]
}

pub(crate) fn sky(sky: &Sky) -> Issues {
    let mut issues = Issues::default();
    for (key, value) in seen(sky) {
        if let Some(value) = value
            && let Some(wrong) = sky_key(key, &value)
                .or_else(|| (!finite(value.parts())).then_some("are finite numbers"))
        {
            let wrong = wrong.strip_prefix("are ").unwrap_or(wrong);
            issues.at(
                &[key],
                format!("sky {key} is {wrong}, not {}", shown(&value)),
            );
        }
    }
    if sky.kind == SkyKind::Mix && keyed(&sky.keys, "intensity") {
        issues.at(
            &["keys", "intensity"],
            "a mix sky keys intensity on its [[sky.layer]]",
        );
    }
    keys(&mut issues, "sky", &sky.keys, Sky::KEYED, sky_key);
    if sky.weight.is_some() {
        issues.at(
            &["weight"],
            "weight belongs to a [[sky.layer]] of a mix sky",
        );
    }
    if !sky.layer.is_empty() && sky.kind != SkyKind::Mix {
        issues.at(&["layer"], "only a mix sky takes [[sky.layer]]");
    }
    if sky.kind != SkyKind::Mix {
        one_sky(&mut issues, sky);
        return issues;
    }
    if sky.path.is_some()
        || sky.prepare.is_some()
        || sky.room.is_some()
        || sky.rotation_deg.is_some()
        || sky.intensity.is_some()
        || sky.turbidity.is_some()
        || sky.ground_albedo.is_some()
        || sky.ambient.is_some()
    {
        issues.at(&["kind"], "a mix sky takes only [[sky.layer]]");
    }
    if sky.layer.len() < 2 {
        issues.at(&["kind"], "a mix sky needs two or more [[sky.layer]]");
    }
    for (place, layer) in sky.layer.iter().enumerate() {
        let mut inner = Issues::default();
        if layer.kind == SkyKind::Mix || !layer.layer.is_empty() {
            inner.at(&["kind"], "a sky layer is analytic, hdr or room, not a mix");
        } else {
            one_sky(&mut inner, layer);
        }
        for (key, value) in seen(layer) {
            if value.is_some() || keyed(&layer.keys, key) {
                let at = if value.is_some() {
                    vec![key]
                } else {
                    vec!["keys", key]
                };
                inner.at(&at, format!("{key} belongs to [sky], not a [[sky.layer]]"));
            }
        }
        keys(&mut inner, "sky layer", &layer.keys, Sky::KEYED, sky_key);
        let weight = layer.weight.unwrap_or(Sky::WEIGHT);
        if !nonnegative(&[weight]) {
            inner.at(
                &["weight"],
                format!("sky layer weight = {weight} must be nonnegative"),
            );
        }
        issues.nested(&["layer".to_string(), place.to_string()], inner);
    }
    issues
}

pub(crate) fn camera(camera: &Camera) -> Issues {
    let mut issues = Issues::default();
    let mut fov = Camera::FOV;
    match camera.projection.unwrap_or_default() {
        Projection::Perspective => {
            if camera.height.is_some() {
                issues.at(
                    &["height"],
                    "a perspective camera takes fov or focal and sensor, not height",
                );
            }
            match (camera.fov, camera.focal) {
                (Some(_), Some(_)) => {
                    issues.at(&["focal"], "a camera takes fov or focal, not both")
                }
                (Some(value), None) => fov = value,
                (None, Some(focal)) => {
                    let sensor = camera.sensor.unwrap_or(Camera::SENSOR);
                    if !positive(focal) || !positive(sensor) {
                        issues.at(&["focal"], "camera focal and sensor must be positive");
                    } else {
                        fov = 2.0 * (sensor / (2.0 * focal)).atan().to_degrees();
                    }
                }
                (None, None) => {
                    if camera.sensor.is_some() {
                        issues.at(&["sensor"], "camera sensor needs focal");
                    }
                }
            }
            if !(fov > 0.0 && fov < 180.0) {
                issues.at(&["fov"], format!("camera fov = {fov} is outside 0 to 180"));
            }
        }
        Projection::Orthographic => {
            if camera.fov.is_some()
                || camera.focal.is_some()
                || camera.sensor.is_some()
                || camera.fstop.is_some()
                || camera.focus.is_some()
                || camera.focus_keys.is_some()
                || ["fov", "fstop", "focus"]
                    .iter()
                    .any(|key| keyed(&camera.keys, key))
            {
                issues.at(&["projection"], "an orthographic camera takes height and shift, not fov, focal, sensor, fstop, focus or focus_keys, nor their keys");
            }
            if !camera.height.is_some_and(positive) {
                issues.at(&[], "an orthographic camera needs a positive height");
            }
        }
    }
    if !camera.shift.is_none_or(|shift| finite(&shift)) {
        issues.at(&["shift"], "camera shift must be finite");
    }
    let at = camera.at.unwrap_or(Camera::AT);
    let look_at = camera.look_at.unwrap_or(Camera::LOOK_AT);
    let up = camera.up.unwrap_or(Camera::UP);
    let fstop = camera.fstop.is_some() || keyed(&camera.keys, "fstop");
    if keyed(&camera.keys, "fov") && camera.focal.is_some() {
        issues.at(&["keys", "fov"], "a camera keys fov with fov, not focal");
    }
    if keyed(&camera.keys, "focus") {
        if camera.focus_keys.is_some() {
            issues.at(
                &["focus_keys"],
                "camera focus_keys and keys.focus are one track; write one",
            );
        }
        if !fstop {
            issues.at(&["keys", "focus"], "camera keys.focus needs fstop");
        }
    }
    keys(
        &mut issues,
        "camera",
        &camera.keys,
        Camera::KEYED,
        |key, value| match (key, value) {
            ("fov", KeyValue::Number(value)) => {
                (!(*value > 0.0 && *value < 180.0)).then_some("are above 0 and below 180")
            }
            ("fstop" | "focus", KeyValue::Number(value)) => above_zero(*value),
            _ => None,
        },
    );
    if let Some(keys) = &camera.focus_keys {
        if !fstop {
            issues.at(&["focus_keys"], "camera focus_keys needs fstop");
        }
        let fine = keys
            .iter()
            .all(|[t, focus]| t.is_finite() && *t >= 0.0 && positive(*focus))
            && keys.windows(2).all(|pair| pair[0][0] <= pair[1][0]);
        if !fine {
            issues.at(
                &["focus_keys"],
                "camera focus_keys are [seconds, distance] pairs, seconds at least 0 and in order, distance above 0",
            );
        }
    }
    if camera.projection.unwrap_or_default() == Projection::Perspective {
        match (camera.fstop, camera.focus) {
            (None, Some(_)) if !fstop => issues.at(&["focus"], "camera focus needs fstop"),
            (None, _) => {}
            (Some(fstop), focus) => {
                let distance =
                    focus.unwrap_or_else(|| dot(sub(look_at, at), sub(look_at, at)).sqrt());
                let sensor = camera.sensor.unwrap_or(Camera::SENSOR);
                let focal = camera
                    .focal
                    .unwrap_or_else(|| sensor / (2.0 * (fov.to_radians() * 0.5).tan()))
                    / 1000.0;
                if !positive(fstop) || !(distance.is_finite() && distance > focal) {
                    issues.at(
                        &["fstop"],
                        "camera fstop must be positive and focus beyond the focal length",
                    );
                }
            }
        }
    }
    let forward = sub(look_at, at);
    let side = cross(forward, up);
    if !finite(&[at, look_at, up].concat())
        || dot(forward, forward) <= 1e-12
        || dot(side, side) <= 1e-12
    {
        issues.at(
            &[],
            "camera at and look_at must differ, and up must not lie along the view",
        );
    }
    let near = camera.near.unwrap_or(Camera::NEAR);
    let far = camera.far.unwrap_or(Camera::FAR);
    if !(positive(near) && far.is_finite() && far > near) {
        issues.at(&[], "camera near must be positive and far beyond it");
    }
    if let Some(preset) = &camera.preset {
        issues.nested(&["preset".to_string()], self::preset(preset));
    }
    issues
}

fn preset(preset: &Preset) -> Issues {
    let mut issues = Issues::default();
    let mut values: Vec<f32> = preset.gain.into_iter().collect();
    if let Some(orbit) = preset.orbit {
        values.extend(
            [orbit.stiffness, orbit.yaw, orbit.pitch, orbit.hold]
                .into_iter()
                .flatten(),
        );
    }
    if let Some(look) = preset.look {
        values.extend(
            [look.stiffness, look.lean, look.yaw, look.pitch, look.sign]
                .into_iter()
                .flatten(),
        );
    }
    if let Some(zoom) = preset.zoom {
        values.extend(
            [zoom.stiffness, zoom.hold, zoom.max, zoom.rate]
                .into_iter()
                .flatten(),
        );
    }
    if let Some(wander) = preset.wander {
        values.extend(
            [wander.idle, wander.every, wander.hold]
                .into_iter()
                .flatten(),
        );
    }
    if let Some(nudge) = preset.nudge {
        values.extend(
            [nudge.stiffness, nudge.pitch, nudge.hold, nudge.floor]
                .into_iter()
                .flatten(),
        );
    }
    if let Some(hush) = preset.hush {
        match (hush.floor, hush.stiffness, hush.blocks_cursor) {
            (Some(floor), Some(stiffness), Some(_)) => values.extend([floor, stiffness]),
            _ => issues.at(
                &["hush"],
                "[camera.preset] hush needs floor, stiffness and blocks_cursor",
            ),
        }
    }
    if let Some(depth) = preset.depth {
        match (depth.focus, depth.blur, depth.floor) {
            (Some(focus), Some(blur), Some(floor)) => values.extend([focus, blur, floor]),
            _ => issues.at(
                &["depth"],
                "[camera.preset] depth needs focus, blur and floor",
            ),
        }
    }
    if !finite(&values) {
        issues.at(&[], "[camera.preset] values must be finite");
    }
    issues
}

pub(crate) fn trace(trace: &Trace) -> Issues {
    let mut issues = Issues::default();
    for (key, value, max) in [
        ("clamp_indirect", trace.clamp_indirect, 10000.0),
        ("filter_glossy", trace.filter_glossy, 1.0),
    ] {
        if let Some(value) = value
            && !(0.0..=max).contains(&value)
        {
            issues.at(
                &[key],
                format!("[trace] {key} = {value} is outside 0 to {max}"),
            );
        }
    }
    issues
}

pub(crate) fn shadows(shadows: &Shadows) -> Issues {
    let mut issues = Issues::default();
    for (key, value, [low, high]) in [
        ("cascades", shadows.cascades, Shadows::CASCADES),
        ("resolution", shadows.resolution, Shadows::RESOLUTION),
    ] {
        if let Some(value) = value
            && !(low..=high).contains(&value)
        {
            issues.at(
                &[key],
                format!("[shadows] {key} = {value} is outside {low} to {high}"),
            );
        }
    }
    if let Some(softness) = shadows.softness
        && !(Shadows::SOFTNESS[0]..=Shadows::SOFTNESS[1]).contains(&softness)
    {
        issues.at(
            &["softness"],
            format!(
                "[shadows] softness = {softness} is outside {} to {}",
                Shadows::SOFTNESS[0],
                Shadows::SOFTNESS[1]
            ),
        );
    }
    for (key, value, [low, high]) in [
        ("bias", shadows.bias, Shadows::BIAS),
        (
            "occlusion_radius",
            shadows.occlusion_radius,
            Shadows::OCCLUSION_RADIUS,
        ),
        (
            "contact_length",
            shadows.contact_length,
            Shadows::CONTACT_LENGTH,
        ),
    ] {
        if let Some(value) = value
            && !(value.is_finite() && value > low && value <= high)
        {
            issues.at(
                &[key],
                format!("[shadows] {key} = {value} is outside {low} (excluded) to {high}"),
            );
        }
    }
    issues
}

pub(crate) fn physics(physics: &Physics) -> Issues {
    let mut issues = Issues::default();
    let rate = physics.rate.unwrap_or(Physics::RATE);
    if !physics.gravity.is_none_or(|gravity| finite(&gravity))
        || !(rate.is_finite() && (1.0..=1000.0).contains(&rate))
        || !(1..=16).contains(&physics.substeps.unwrap_or(Physics::SUBSTEPS))
        || !(1..=64).contains(&physics.iterations.unwrap_or(Physics::ITERATIONS))
    {
        issues.at(
            &[],
            "[physics] needs a finite gravity, a rate of 1 to 1000, substeps of 1 to 16 and iterations of 1 to 64",
        );
    }
    issues
}

pub(crate) fn plates(plates: &Plates) -> Issues {
    let mut issues = Issues::default();
    if plates.casters == Some(Casters::Proxies) && plates.proxies.is_none() {
        issues.at(
            &["casters"],
            "[plates] casters = \"proxies\" needs a proxies file in proxies",
        );
    }
    issues
}

pub(crate) fn proxy(proxy: &Proxy) -> Issues {
    let mut issues = Issues::default();
    let object = &proxy.object;
    match proxy.kind {
        ProxyKind::Box => {
            for (key, present) in [
                ("points", proxy.points.is_some()),
                ("triangles", proxy.triangles.is_some()),
            ] {
                if present {
                    issues.at(
                        &[key],
                        format!("proxy for {object}: a box takes at, rotate and size, not {key}"),
                    );
                }
            }
            if !finite(&proxy.at.unwrap_or_default()) || !finite(&proxy.rotate.unwrap_or_default())
            {
                issues.at(
                    &[],
                    format!("proxy for {object}: a box needs a finite at and rotate"),
                );
            }
            match proxy.size {
                None => issues.at(
                    &[],
                    format!("missing key 'size': proxy for {object} is a box, which needs a size"),
                ),
                Some(size) if !size.iter().all(|value| positive(*value)) => issues.at(
                    &["size"],
                    format!("proxy for {object}: each value of a box's size is above 0"),
                ),
                Some(_) => {}
            }
        }
        ProxyKind::Hull => {
            for (key, present) in [
                ("at", proxy.at.is_some()),
                ("rotate", proxy.rotate.is_some()),
                ("size", proxy.size.is_some()),
            ] {
                if present {
                    issues.at(
                        &[key],
                        format!("proxy for {object}: a hull takes points and triangles, not {key}"),
                    );
                }
            }
            match &proxy.points {
                None => issues.at(
                    &[],
                    format!(
                        "missing key 'points': proxy for {object} is a hull, which needs points"
                    ),
                ),
                Some(points) if points.len() < 4 => issues.at(
                    &["points"],
                    format!(
                        "proxy for {object}: a hull has at least four points, not {}",
                        points.len()
                    ),
                ),
                Some(points) if !finite(&points.concat()) => issues.at(
                    &["points"],
                    format!("proxy for {object}: a hull has a point that is not finite"),
                ),
                Some(_) => {}
            }
            match &proxy.triangles {
                None => issues.at(
                    &[],
                    format!(
                        "missing key 'triangles': proxy for {object} is a hull, which needs triangles"
                    ),
                ),
                Some(triangles) if triangles.len() < 4 => issues.at(
                    &["triangles"],
                    format!(
                        "proxy for {object}: a hull has at least four triangles, not {}",
                        triangles.len()
                    ),
                ),
                Some(triangles) => {
                    let count = proxy.points.as_ref().map_or(0, Vec::len);
                    if let Some((place, corner)) = triangles
                        .iter()
                        .enumerate()
                        .find_map(|(place, triangle)| {
                            triangle
                                .iter()
                                .find(|corner| **corner as usize >= count)
                                .map(|corner| (place, *corner))
                        })
                        && proxy.points.is_some()
                    {
                        issues.at(
                            &["triangles", &place.to_string()],
                            format!(
                                "proxy for {object}: triangle {place} names point {corner}, and the hull has {count} points, counted from 0"
                            ),
                        );
                    }
                }
            }
        }
    }
    issues
}

fn color(issues: &mut Issues, key: &str, color: &Option<Color>) {
    if let Some(color) = color
        && !color.valid()
    {
        issues.at(&[key], format!("{key} is #rrggbb or three numbers"));
    }
}

fn infinite(value: &toml::Value) -> bool {
    match value {
        toml::Value::Float(number) => !number.is_finite(),
        toml::Value::Array(items) => items.iter().any(infinite),
        toml::Value::Table(table) => table.values().any(infinite),
        _ => false,
    }
}

pub(crate) fn finish_keys(keys: &FinishKeys) -> Issues {
    let mut issues = Issues::default();
    for (key, value) in keys.to_table() {
        if infinite(&value) {
            issues.at(&[&key], format!("{key} is a finite number"));
        }
    }
    if let Some(steps) = keys.tone_steps
        && !(3.0..=5.0).contains(&steps.round())
    {
        issues.at(&["tone_steps"], "tone_steps is 3 to 5");
    }
    if let Some(bands) = keys.cel_bands
        && bands < 0.0
    {
        issues.at(&["cel_bands"], "cel_bands is a positive number");
    }
    if let Some(Tone::Table(table)) = &keys.tone
        && table.kind != ToneKind::Neutral
        && (table.start.is_some() || table.clamp.is_some())
    {
        issues.at(&["tone"], "tone.start and tone.clamp are for neutral");
    }
    if let Some(crate::types::Bloom::Table(bloom)) = &keys.bloom {
        for (key, value) in [("radius", bloom.radius), ("down", bloom.down)] {
            if value.is_some_and(|value| value < 0.0) {
                issues.at(&["bloom", key], format!("bloom.{key} is a positive number"));
            }
        }
    }
    if let Some(LutKey::Table(lut)) = &keys.lut {
        if !Lut::SIZES.contains(&lut.size) {
            issues.at(&["lut", "size"], "lut.size is from 2 to 64");
        } else if lut.values.len() != lut.needs() {
            issues.at(
                &["lut", "values"],
                format!("lut.values has {} numbers", lut.needs()),
            );
        }
    }
    unit(&mut issues, &["lut_amount"], keys.lut_amount);
    if let Some(Tape::Strength(value)) = keys.tape
        && !nonnegative(&[value])
    {
        issues.at(&["tape"], "tape is a number of at least 0");
    }
    if let Some(Tape::Table(tape)) = &keys.tape {
        let values = [
            tape.bands,
            tape.echo,
            tape.grain,
            tape.smear,
            tape.split,
            tape.strength,
            tape.wash,
        ];
        if !nonnegative(&values.into_iter().flatten().collect::<Vec<_>>()) {
            issues.at(&["tape"], "tape values are numbers of at least 0");
        }
    }
    if let Some(Scratches::Table(table)) = &keys.scratches
        && table.strength.is_some_and(|value| !value.is_finite())
    {
        issues.at(&["scratches"], "scratches.strength is a number");
    }
    for (key, value) in [
        ("rim_color", &keys.rim_color),
        ("outline_color", &keys.outline_color),
        ("keep", &keys.keep),
        ("shadow", &keys.shadow),
        ("highlight", &keys.highlight),
    ] {
        color(&mut issues, key, value);
    }
    if let Some(Ink::Color(value)) = &keys.ink {
        color(&mut issues, "ink", &Some(value.clone()));
    }
    if let Some(crate::types::Outline::Table(table)) = &keys.outline {
        color(&mut issues, "outline", &table.color);
    }
    issues
}

pub(crate) fn builds_nothing(keys: &FinishKeys) -> bool {
    let mut rest = keys.clone();
    if rest.encode == Some(false) {
        rest.encode = None;
    }
    rest.is_empty()
}

pub(crate) fn finish(finish: &Finish) -> Issues {
    let mut issues = Issues::default();
    let keys = finish.keys();
    if finish.file.is_some() && !keys.is_empty() {
        issues.at(
            &["file"],
            "[finish] takes file alone, or the finish keys inline, with [[finish.pass]] after either",
        );
    }
    issues.0.extend(finish_keys(&keys).0);
    self::keys(
        &mut issues,
        "finish",
        &finish.keys,
        Finish::KEYED,
        |key, value| match (key, value) {
            ("lut_amount", KeyValue::Number(value)) => unit_of(*value),
            _ => None,
        },
    );
    for (place, pass) in finish.pass.iter().enumerate() {
        let at = ["pass".to_string(), place.to_string()];
        let mut inner = Issues::default();
        for (key, present) in [
            ("style", pass.style.is_some()),
            ("seed", pass.seed.is_some()),
            ("frame", pass.frame.is_some()),
        ] {
            if present {
                inner.at(
                    &[key],
                    format!("a [[finish.pass]] takes finish keys, not {key}; set it on [finish]"),
                );
            }
        }
        if builds_nothing(pass) {
            inner.at(&[], "a [[finish.pass]] builds no pass");
        }
        inner.0.extend(finish_keys(pass).0);
        issues.nested(&at, inner);
    }
    issues
}

pub(crate) fn material(name: &str, material: &Material) -> Issues {
    let mut issues = Issues::default();
    if let Some(wet) = &material.wet {
        unit(&mut issues, &["wet", "amount"], wet.amount);
        unit(&mut issues, &["wet", "puddles"], wet.puddles);
        if let Some(scale) = wet.puddle_scale
            && !positive(scale)
        {
            issues.at(
                &["wet", "puddle_scale"],
                format!("material {name}: wet.puddle_scale is above 0"),
            );
        }
    }
    if let Some(fabric) = &material.fabric {
        unit_color(&mut issues, &["fabric", "color"], fabric.color);
        unit(&mut issues, &["fabric", "roughness"], fabric.roughness);
    }
    if let Some(anisotropy) = &material.anisotropy {
        unit(
            &mut issues,
            &["anisotropy", "strength"],
            anisotropy.strength,
        );
        if let Some(rotation) = anisotropy.rotation
            && !rotation.is_finite()
        {
            issues.at(
                &["anisotropy", "rotation"],
                format!("material {name}: anisotropy.rotation is a finite number of radians"),
            );
        }
    }
    if let Some(scatter) = &material.scatter {
        unit_color(&mut issues, &["scatter", "color"], scatter.color);
        unit(&mut issues, &["scatter", "amount"], scatter.amount);
        if let Some(radius) = scatter.radius
            && !positive(radius)
        {
            issues.at(
                &["scatter", "radius"],
                format!("material {name}: scatter.radius is above 0"),
            );
        }
    }
    if material.layers.len() > LAYER_CAP {
        issues.at(
            &["layers"],
            format!(
                "material {name} takes at most {LAYER_CAP} noise layers, not {}",
                material.layers.len()
            ),
        );
    }
    issues
}

pub(crate) fn tunable(name: &str, tunable: &Tunable) -> Issues {
    let mut issues = Issues::default();
    let kind = tunable.kind;
    for (key, value) in [
        ("default", Some(tunable.default)),
        ("min", tunable.min),
        ("max", tunable.max),
    ] {
        let Some(value) = value else { continue };
        if key != "default" && kind == TunableKind::Bool {
            issues.at(
                &[key],
                format!("tunable {name} is a bool, which takes no {key}"),
            );
        } else if value.kind() != kind {
            let (article, wanted) = match kind {
                TunableKind::Float => ("a", "a number"),
                TunableKind::Int => ("an", "a whole number"),
                TunableKind::Bool => ("a", "true or false"),
                TunableKind::Vector => ("a", "[x, y, z]"),
            };
            issues.typed(
                &[key],
                format!(
                    "tunable {name} is {article} {}, so its {key} is {wanted}, not {value}",
                    kind.name()
                ),
            );
        } else if !value.finite() {
            issues.at(
                &[key],
                format!("tunable {name}: {key} = {value} is not finite"),
            );
        }
    }
    if !issues.0.is_empty() {
        return issues;
    }
    if let (Some(min), Some(max)) = (tunable.min, tunable.max)
        && let Some(axis) = past(min, max, Ordering::Greater)
    {
        issues.at(
            &["min"],
            format!("tunable {name}: min {min} is above max {max}{axis}"),
        );
        return issues;
    }
    for (bound, limit, side, word) in [
        ("min", tunable.min, Ordering::Less, "below"),
        ("max", tunable.max, Ordering::Greater, "above"),
    ] {
        if let Some(limit) = limit
            && let Some(axis) = past(tunable.default, limit, side)
        {
            issues.at(
                &["default"],
                format!(
                    "tunable {name}: default {} is {word} {bound} {limit}{axis}",
                    tunable.default
                ),
            );
        }
    }
    issues
}

fn past(value: TunableValue, limit: TunableValue, side: Ordering) -> Option<&'static str> {
    let order = |a: f32, b: f32| a.partial_cmp(&b).unwrap_or(Ordering::Equal);
    match (value, limit) {
        (TunableValue::Int(a), TunableValue::Int(b)) => (a.cmp(&b) == side).then_some(""),
        (TunableValue::Float(a), TunableValue::Float(b)) => (order(a, b) == side).then_some(""),
        (TunableValue::Vector(a), TunableValue::Vector(b)) => [" in x", " in y", " in z"]
            .into_iter()
            .zip(a.into_iter().zip(b))
            .find(|(_, (a, b))| order(*a, *b) == side)
            .map(|(axis, _)| axis),
        _ => None,
    }
}
