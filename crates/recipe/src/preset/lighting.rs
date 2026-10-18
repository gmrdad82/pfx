use std::path::Path;

use pfx_mesh::math::Real;
use toml_edit::{ArrayOfTables, Item, Table};

use super::{ABOVE_0, ANY, FROM_0, Params, num, parsed, point, quote};
use crate::include::Sources;
use crate::{CAP_LUMINANCE, CAP_MEAN, Error};

pub(crate) const KINDS: [&str; 4] = ["three-point", "golden-hour", "overcast", "night"];

pub(crate) const HDRI_DIR: &str = "assets/sky";

struct Plan {
    sun_az: f64,
    sun_el: f64,
    sun_color: [f64; 3],
    sun_radius: f64,
    sun_strength: f64,
    sky_analytic: f64,
    sky_hdr: f64,
    turbidity: f64,
    hdri: &'static str,
    exposure: f64,
    grade: [f64; 5],
}

fn plan(kind: &str) -> Plan {
    match kind {
        "three-point" => Plan {
            sun_az: 0.0,
            sun_el: 45.0,
            sun_color: [1.0, 1.0, 1.0],
            sun_radius: 0.27,
            sun_strength: 0.0,
            sky_analytic: 0.25,
            sky_hdr: 0.7,
            turbidity: 3.0,
            hdri: "studio_small_03",
            exposure: 1.0,
            grade: [0.1, 1.0, 0.0, 0.05, 0.0],
        },
        "golden-hour" => Plan {
            sun_az: 45.0,
            sun_el: 8.0,
            sun_color: [1.0, 0.62, 0.34],
            sun_radius: 0.8,
            sun_strength: 3.5,
            sky_analytic: 0.45,
            sky_hdr: 1.0,
            turbidity: 4.0,
            hdri: "venice_sunset",
            exposure: 1.0,
            grade: [0.18, 1.1, 0.12, 0.12, 0.0],
        },
        "overcast" => Plan {
            sun_az: 0.0,
            sun_el: 65.0,
            sun_color: [0.92, 0.95, 1.0],
            sun_radius: 12.0,
            sun_strength: 1.0,
            sky_analytic: 1.1,
            sky_hdr: 1.0,
            turbidity: 10.0,
            hdri: "kloofendal_overcast_puresky",
            exposure: 1.1,
            grade: [0.08, 0.92, -0.03, 0.0, 0.0],
        },
        _ => Plan {
            sun_az: 200.0,
            sun_el: 40.0,
            sun_color: [0.55, 0.68, 1.0],
            sun_radius: 0.5,
            sun_strength: 0.1,
            sky_analytic: 0.02,
            sky_hdr: 1.0,
            turbidity: 3.0,
            hdri: "moonlit_golf",
            exposure: 1.5,
            grade: [0.3, 0.85, -0.08, 0.25, 0.0],
        },
    }
}

fn toward(azimuth: f64, elevation: f64) -> [f64; 3] {
    let (sa, ca) = azimuth.to_radians().sin_cos_of();
    let (se, ce) = elevation.to_radians().sin_cos_of();
    [sa * ce, se, ca * ce]
}

struct Lamp {
    name: &'static str,
    azimuth: f64,
    elevation: f64,
    reach: f64,
    color: [f64; 3],
    strength: f64,
    radius: f64,
    shadow: bool,
}

fn lamps(kind: &str) -> Vec<Lamp> {
    match kind {
        "three-point" => vec![
            Lamp {
                name: "rig_key",
                azimuth: 40.0,
                elevation: 35.0,
                reach: 1.0,
                color: [1.0, 0.95, 0.88],
                strength: 1.8,
                radius: 0.15,
                shadow: true,
            },
            Lamp {
                name: "rig_fill",
                azimuth: -55.0,
                elevation: 10.0,
                reach: 1.0,
                color: [0.85, 0.92, 1.0],
                strength: 0.56,
                radius: 0.3,
                shadow: false,
            },
            Lamp {
                name: "rig_back",
                azimuth: 160.0,
                elevation: 40.0,
                reach: 1.0,
                color: [1.0, 1.0, 1.0],
                strength: 1.2,
                radius: 0.05,
                shadow: false,
            },
        ],
        "golden-hour" => vec![Lamp {
            name: "golden_bounce",
            azimuth: 225.0,
            elevation: 5.0,
            reach: 1.0,
            color: [1.0, 0.72, 0.5],
            strength: 0.12,
            radius: 0.5,
            shadow: false,
        }],
        "night" => vec![
            Lamp {
                name: "night_lamp",
                azimuth: 90.0,
                elevation: 40.0,
                reach: 0.8,
                color: [1.0, 0.68, 0.38],
                strength: 0.5,
                radius: 0.08,
                shadow: true,
            },
            Lamp {
                name: "night_rim",
                azimuth: 160.0,
                elevation: 25.0,
                reach: 1.0,
                color: [0.45, 0.6, 1.0],
                strength: 0.1,
                radius: 0.05,
                shadow: false,
            },
        ],
        _ => Vec::new(),
    }
}

pub(super) fn expand(root: &mut Table, src: &Sources, base: Option<&Path>) -> Result<(), Error> {
    let Some(item) = root.get("lighting") else {
        return Ok(());
    };
    let line = item.span().map(|span| src.line(span.start));
    let Some(table) = item.as_table_like() else {
        return Err(Error::new(
            "lighting",
            line,
            "expected a table with `preset`, as `[lighting]`",
        ));
    };
    let mut p = Params::new(src, table, "lighting".to_string(), line);
    let kind = p.choice("preset", "", &KINDS)?;
    if kind.is_empty() {
        return Err(p.missing("preset", &format!("one of: {}", KINDS.join(", "))));
    }
    let plan = plan(kind);
    let subject = p.point("subject", [0.0, 1.0, 0.0])?;
    let distance = p.number("distance", 3.0, ABOVE_0)?;
    let strength = p.number("intensity", 1.0, FROM_0)?;
    let azimuth = p.number("azimuth_deg", plan.sun_az, ANY)?;
    let elevation = p.number("elevation_deg", plan.sun_el, super::PITCH)?;
    let rotation = p.number("rotation_deg", 0.0, ANY)?;
    let hdri = p.text("hdri")?.unwrap_or(plan.hdri).to_string();
    let dir = p.text("hdri_dir")?.unwrap_or(HDRI_DIR).to_string();
    p.finish()?;

    let found = hdri != "none"
        && base.is_some_and(|base| {
            crate::files::is_file(&base.join(&dir).join(format!("{hdri}.hdr")))
        });
    let mut text = String::new();
    if found {
        text += &format!(
            "[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\nirradiance = 0.0\n\n[sky]\nkind = \"hdr\"\npath = {}\nsource = {}\nrotation_deg = {}\nintensity = {}\n\n[sky.prepare]\ncap = {}\nmean = {}\n\n",
            quote(&format!("{dir}/{hdri}.hdr")),
            quote(&format!("polyhaven:{hdri}, CC0")),
            num(rotation),
            num(plan.sky_hdr * strength),
            num(CAP_LUMINANCE),
            num(CAP_MEAN),
        );
    } else {
        text += &format!(
            "[sun]\nmodel = \"authored\"\ntoward = {}\nradius_deg = {}\ncolor = {}\nirradiance = {}\n\n[sky]\nkind = \"analytic\"\nintensity = {}\nturbidity = {}\nrotation_deg = {}\n\n",
            point(toward(azimuth, elevation)),
            num(plan.sun_radius),
            point(plan.sun_color),
            num(plan.sun_strength * strength),
            num(plan.sky_analytic * strength),
            num(plan.turbidity),
            num(rotation),
        );
    }
    let [vignette, saturation, warmth, bloom, haze] = plan.grade;
    text += &format!(
        "[look]\nexposure = {}\ngrade = {{ vignette = {}, saturation = {}, warmth = {}, bloom = {}, haze = {} }}\n",
        num(plan.exposure),
        num(vignette),
        num(saturation),
        num(warmth),
        num(bloom),
        num(haze),
    );
    let swing = azimuth - plan.sun_az;
    for lamp in lamps(kind) {
        let reach = distance * lamp.reach;
        let at = toward(lamp.azimuth + swing, lamp.elevation);
        let place = [
            subject[0] + at[0] * reach,
            subject[1] + at[1] * reach,
            subject[2] + at[2] * reach,
        ];
        text += &format!(
            "\n[[light]]\nname = {}\nposition = {}\ncolor = {}\nintensity = {}\nradius = {}\nshadow = {}\n",
            quote(lamp.name),
            point(place),
            point(lamp.color),
            num(lamp.strength * distance * distance * strength),
            num(lamp.radius),
            lamp.shadow,
        );
    }
    let built = parsed(&text, "lighting")?;
    root.remove("lighting");
    for key in ["sun", "sky", "look"] {
        if !root.contains_key(key)
            && let Some(item) = built.get(key)
        {
            root.insert(key, item.clone());
        }
    }
    let Some(lights) = built.get("light").and_then(Item::as_array_of_tables) else {
        return Ok(());
    };
    if !root.contains_key("light") {
        root.insert("light", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let Some(have) = root.get_mut("light").and_then(Item::as_array_of_tables_mut) else {
        return Err(Error::new(
            "light",
            line,
            "a lighting preset adds `[[light]]` tables, expected the recipe's lights written as `[[light]]`",
        ));
    };
    for light in lights.iter() {
        let name = light.get("name").and_then(Item::as_str);
        if !have
            .iter()
            .any(|l| l.get("name").and_then(Item::as_str) == name)
        {
            have.push(light.clone());
        }
    }
    Ok(())
}
