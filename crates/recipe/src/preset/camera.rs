use pfx_mesh::math::Real;
use toml_edit::{Item, Table};

use super::{
    ABOVE_0, ANY, Clock, FROM_0, PITCH, Params, ROLL, fill, negated, num, number_of, parsed, point,
    quote,
};
use crate::Error;
use crate::include::Sources;

pub(crate) const KINDS: [&str; 6] = ["chase", "orbit", "dolly", "crane", "drone", "drive-by"];

const FOLLOW: &str = "\"follow\"";

pub(super) fn expand(root: &mut Table, src: &Sources, clock: &Clock) -> Result<(), Error> {
    let Some(cameras) = root
        .get_mut("camera")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return Ok(());
    };
    for (index, camera) in cameras.iter_mut().enumerate() {
        let Some(item) = camera.get("preset") else {
            continue;
        };
        let label = camera
            .get("name")
            .and_then(Item::as_str)
            .map_or_else(|| index.to_string(), str::to_string);
        let path = format!("camera[{label}].preset");
        let line = item.span().map(|span| src.line(span.start));
        let Some(table) = item.as_table_like() else {
            return Err(Error::new(
                path,
                line,
                "expected a table with `kind`, as `[camera.preset]`",
            ));
        };
        if camera.get("kind").and_then(Item::as_str) == Some("orthographic") {
            return Err(Error::new(
                path,
                line,
                "a preset moves a perspective camera, expected the camera's `kind` removed or \"perspective\"",
            ));
        }
        let deep = camera
            .get("fstop")
            .and_then(number_of)
            .is_some_and(|fstop| fstop > 0.0);
        let mut p = Params::new(src, table, path.clone(), line);
        let text = build(&mut p, clock, deep)?;
        p.finish()?;
        let built = parsed(&text, &path)?.as_table().clone();
        camera.remove("preset");
        fill(camera, &built);
    }
    Ok(())
}

fn build(p: &mut Params<'_>, clock: &Clock, deep: bool) -> Result<String, Error> {
    let kind = p.choice("kind", "", &KINDS)?;
    match kind {
        "chase" => chase(p, deep),
        "orbit" => orbit(p),
        "dolly" => dolly(p),
        "crane" => crane(p, clock, deep),
        "drone" => drone(p, clock, deep),
        "drive-by" => drive_by(p, deep),
        _ => Err(p.missing("kind", &format!("one of: {}", KINDS.join(", ")))),
    }
}

fn focus(p: &mut Params<'_>, deep: bool) -> Result<Option<String>, Error> {
    let Some(item) = p.get("focus") else {
        return Ok(deep.then(|| FOLLOW.to_string()));
    };
    if let Some(on) = item.as_bool() {
        return Ok(on.then(|| FOLLOW.to_string()));
    }
    if item.as_str() == Some("follow") {
        return Ok(Some(FOLLOW.to_string()));
    }
    match number_of(item).filter(|m| m.is_finite() && *m > 0.0) {
        Some(metres) => Ok(Some(num(metres))),
        None => Err(p.refuse(
            "focus",
            item,
            "expected true, false, \"follow\" or a distance in metres above 0",
        )),
    }
}

fn seconds(p: &mut Params<'_>, clock: &Clock) -> Result<f64, Error> {
    match p.maybe("seconds", ABOVE_0)?.or(clock.seconds) {
        Some(seconds) => Ok(seconds),
        None => Err(p.missing("seconds", "the move's length, or a `[timeline]` with one")),
    }
}

fn around(distance: f64, orbit_deg: f64, pitch_deg: f64) -> [f64; 3] {
    let (sa, ca) = orbit_deg.to_radians().sin_cos_of();
    let (se, ce) = pitch_deg.to_radians().sin_cos_of();
    [-distance * ce * sa, distance * se, -distance * ce * ca]
}

fn rig(follow: &str, offset: [f64; 3], look_height: f64) -> String {
    format!(
        "[rig]\nfollow = {}\noffset = {}\nlook = {}\n",
        quote(follow),
        point(offset),
        point([0.0, look_height, 0.0]),
    )
}

fn chase(p: &mut Params<'_>, deep: bool) -> Result<String, Error> {
    let follow = p.name("follow", "the name of the object to chase")?;
    let distance = p.number("distance", 5.5, ABOVE_0)?;
    let height = p.number("height", 1.1, ANY)?;
    let lateral = p.number("lateral", 0.0, ANY)?;
    let look_height = p.number("look_height", 0.6, ANY)?;
    let lag = p.number("lag", 0.25, FROM_0)?;
    let look_ahead = p.number("look_ahead", 0.3, FROM_0)?;
    let yaw_lag = p.number("yaw_lag", 0.4, FROM_0)?;
    let bank = p.number("bank", 4.0, FROM_0)?;
    let focus = focus(p, deep)?;
    let mut out = rig(
        follow,
        [negated(lateral), height, negated(distance)],
        look_height,
    );
    out += &format!(
        "lag = {}\nlook_ahead = {}\nyaw_lag = {}\nbank = {}\n",
        num(lag),
        num(look_ahead),
        num(yaw_lag),
        num(bank),
    );
    if let Some(focus) = focus {
        out += &format!("focus = {focus}\n");
    }
    Ok(out)
}

fn orbit(p: &mut Params<'_>) -> Result<String, Error> {
    let target = p.point("target", [0.0, 1.0, 0.0])?;
    let radius = p.number("radius", 3.0, ABOVE_0)?;
    let height = p.number("height", 1.5, ANY)?;
    let azimuth = p.number("azimuth_deg", 0.0, ANY)?;
    let turns = p.number("turns", 1.0, ANY)?;
    let rise = p.number("rise_deg", 0.0, ANY)?;
    let scale = p.number("radius_scale", 1.0, ABOVE_0)?;
    let ease = p.choice("ease", "linear", &["linear", "smooth"])?;
    let (sa, ca) = azimuth.to_radians().sin_cos_of();
    let position = [target[0] + radius * sa, height, target[2] + radius * ca];
    Ok(format!(
        "position = {}\ntarget = {}\n\n[path]\nkind = \"orbit\"\ntarget = {}\nturns = {}\nrise_deg = {}\nradius_scale = {}\nease = {}\n",
        point(position),
        point(target),
        point(target),
        num(turns),
        num(rise),
        num(scale),
        quote(ease),
    ))
}

fn dolly(p: &mut Params<'_>) -> Result<String, Error> {
    let from = p.point("from", [0.0, 1.2, 4.0])?;
    let to = p.point("to", [0.0, 1.2, 2.0])?;
    let target = p.point("target", [0.0, 1.0, 0.0])?;
    let target_to = p.point("target_to", target)?;
    let focal = p.pair("focal_mm")?;
    if let Some(item) = p.table.get("focal_mm")
        && focal.is_some_and(|f| f.iter().any(|v| *v <= 0.0))
    {
        return Err(p.refuse("focal_mm", item, "expected a lens above 0"));
    }
    let ease = p.choice("ease", "smooth", &["linear", "smooth"])?;
    let sensor = focal.map_or(String::new(), |_| {
        format!("sensor_mm = {}\n", num(crate::SENSOR_MM))
    });
    let lens = |at: usize| focal.map_or(String::new(), |f| format!("focal_mm = {}\n", num(f[at])));
    Ok(format!(
        "position = {}\ntarget = {}\n{}{}\n[path]\nkind = \"keys\"\nease = {}\n\n[[path.keys]]\nt = 0.0\nposition = {}\ntarget = {}\n{}\n[[path.keys]]\nt = 1.0\nposition = {}\ntarget = {}\n{}",
        point(from),
        point(target),
        lens(0),
        sensor,
        quote(ease),
        point(from),
        point(target),
        lens(0),
        point(to),
        point(target_to),
        lens(1),
    ))
}

fn crane(p: &mut Params<'_>, clock: &Clock, deep: bool) -> Result<String, Error> {
    let follow = p.name("follow", "the name of the object to follow")?;
    let length = seconds(p, clock)?;
    let from_pitch = p.number("from_pitch", 5.0, PITCH)?;
    let to_pitch = p.number("to_pitch", 50.0, PITCH)?;
    let from_distance = p.number("from_distance", 5.0, ABOVE_0)?;
    let to_distance = p.number("to_distance", 9.0, ABOVE_0)?;
    let azimuth = p.number("azimuth_deg", 0.0, ANY)?;
    let look_height = p.number("look_height", 0.6, ANY)?;
    let smooth = p.number("smooth", 0.0, FROM_0)?;
    let focus = focus(p, deep)?;
    let mut out = rig(
        follow,
        around(from_distance, azimuth, from_pitch),
        look_height,
    );
    if let Some(focus) = focus {
        out += &format!("focus = {focus}\n");
    }
    if smooth > 0.0 {
        out += &format!(
            "smooth = {{ pitch = {s}, distance = {s} }}\n",
            s = num(smooth)
        );
    }
    out += &format!(
        "\n[rig.tracks]\ndistance = [[0.0, {}], [{}, {}]]\npitch = [[0.0, {}], [{}, {}]]\n",
        num(from_distance),
        num(length),
        num(to_distance),
        num(from_pitch),
        num(length),
        num(to_pitch),
    );
    if azimuth != 0.0 {
        out += &format!("orbit = [[0.0, {}]]\n", num(azimuth));
    }
    Ok(out)
}

fn drone(p: &mut Params<'_>, clock: &Clock, deep: bool) -> Result<String, Error> {
    let follow = p.name("follow", "the name of the object to circle")?;
    let length = seconds(p, clock)?;
    let distance = p.number("distance", 12.0, ABOVE_0)?;
    let from_pitch = p.number("from_pitch", 20.0, PITCH)?;
    let to_pitch = p.number("to_pitch", 35.0, PITCH)?;
    let turns = p.number("turns", 0.5, ANY)?;
    let azimuth = p.number("azimuth_deg", 0.0, ANY)?;
    let look_height = p.number("look_height", 0.6, ANY)?;
    let drift = p.number("drift", 0.4, FROM_0)?;
    let drift_roll = p.number("drift_roll", 0.3, ROLL)?;
    let drift_hz = p.number("drift_hz", 0.35, ABOVE_0)?;
    let seed = p.whole("seed")?;
    let focus = focus(p, deep)?;
    let mut out = rig(follow, around(distance, azimuth, from_pitch), look_height);
    out += "frame = \"world\"\n";
    if drift > 0.0 || drift_roll > 0.0 {
        let seed = seed.map_or(String::new(), |seed| format!(", seed = {seed}"));
        out += &format!(
            "shake = {{ translation = {}, roll = {}, frequency = {}{seed} }}\n",
            num(drift),
            num(drift_roll),
            num(drift_hz),
        );
    }
    if let Some(focus) = focus {
        out += &format!("focus = {focus}\n");
    }
    let orbit: Vec<String> = (0..=4)
        .map(|i| {
            let step = f64::from(i) / 4.0;
            format!(
                "[{}, {}]",
                num(length * step),
                num(azimuth + 360.0 * turns * step)
            )
        })
        .collect();
    out += &format!(
        "\n[rig.tracks]\ndistance = [[0.0, {}]]\npitch = [[0.0, {}], [{}, {}]]\norbit = [{}]\n",
        num(distance),
        num(from_pitch),
        num(length),
        num(to_pitch),
        orbit.join(", "),
    );
    Ok(out)
}

fn drive_by(p: &mut Params<'_>, deep: bool) -> Result<String, Error> {
    let follow = p.name("follow", "the name of the object to watch pass")?;
    let side = p.number("side", 4.0, ANY)?;
    let ahead = p.number("ahead", 25.0, ANY)?;
    let height = p.number("height", 1.4, ANY)?;
    let look_height = p.number("look_height", 0.5, ANY)?;
    let release = p.number("release", 0.0, FROM_0)?;
    let focus = focus(p, deep)?;
    let mut out = rig(follow, [negated(side), height, ahead], look_height);
    out += &format!("release = {}\nrelease_ease = 0.001\n", num(release));
    if let Some(focus) = focus {
        out += &format!("focus = {focus}\n");
    }
    Ok(out)
}
