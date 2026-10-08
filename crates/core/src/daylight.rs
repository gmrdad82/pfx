/// Noon. A caller passes it to [`Daylight::light`] and [`Daylight::authored`] for a neutral normaliser.
pub const REFERENCE_HOUR: f64 = 12.0;
const AXIAL_TILT: f64 = 23.44;

const FIELDS: [(&str, f64, f64, Option<f64>); 4] = [
    ("hour", 0.0, 24.0, None),
    ("day", 1.0, 365.0, Some(Daylight::DAY)),
    ("latitude", -90.0, 90.0, Some(Daylight::LATITUDE)),
    ("heading", 0.0, 360.0, Some(Daylight::HEADING)),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    Number(f64),
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Daylight {
    pub hour: f64,
    pub day: f64,
    pub latitude: f64,
    pub heading: f64,
}

impl Daylight {
    pub const DAY: f64 = 172.0;
    pub const LATITUDE: f64 = 45.0;
    pub const HEADING: f64 = 180.0;

    pub fn parse<I, K>(fields: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (K, Reading)>,
        K: AsRef<str>,
    {
        let fields: Vec<(String, Reading)> = fields
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_string(), value))
            .collect();
        for (key, _) in &fields {
            if !FIELDS.iter().any(|field| field.0 == key) {
                return Err(format!(
                    "daylight knows no '{key}'; it takes hour, day, latitude and heading"
                ));
            }
        }
        let mut found = [None, None, None, None];
        for (key, value) in &fields {
            if let Some(index) = FIELDS.iter().position(|field| field.0 == key) {
                found[index] = Some(value);
            }
        }
        let mut values = [0.0; 4];
        for (index, &(key, low, high, fallback)) in FIELDS.iter().enumerate() {
            let value = match found[index] {
                Some(Reading::Number(value)) => *value,
                Some(Reading::Other(text)) => {
                    return Err(format!("daylight.{key} is {text}, not a number"));
                }
                None => fallback.ok_or("daylight needs an hour, 0 to 24")?,
            };
            if !(low..=high).contains(&value) {
                return Err(format!(
                    "daylight.{key} = {value} is outside {low} to {high}"
                ));
            }
            values[index] = value;
        }
        Ok(Self {
            hour: values[0],
            day: values[1],
            latitude: values[2],
            heading: values[3],
        })
    }

    pub fn sun(&self) -> Sun {
        let declination =
            (AXIAL_TILT * (std::f64::consts::TAU * (284.0 + self.day) / 365.0).sin()).to_radians();
        let hour_angle = (15.0 * (self.hour - 12.0)).to_radians();
        let latitude = self.latitude.to_radians();
        let elevation = (latitude.sin() * declination.sin()
            + latitude.cos() * declination.cos() * hour_angle.cos())
        .clamp(-1.0, 1.0)
        .asin();
        let bearing = (-declination.cos() * hour_angle.sin()).atan2(
            declination.sin() * latitude.cos()
                - declination.cos() * hour_angle.cos() * latitude.sin(),
        );
        let flat = elevation.cos();
        let front = self.heading.to_radians();
        let right = front - std::f64::consts::FRAC_PI_2;
        let z_up = [
            flat * (bearing - right).cos(),
            -flat * (bearing - front).cos(),
            elevation.sin(),
        ];
        Sun {
            elevation: elevation.to_degrees(),
            azimuth: bearing.to_degrees().rem_euclid(360.0),
            y_up: [z_up[0], z_up[2], -z_up[1]],
            z_up,
        }
    }

    pub fn light(&self, reference_hour: f64) -> Light {
        let at = self.sun();
        let reference = reference_place(reference_hour).sun();
        let colour = kelvin(temperature(at.elevation));
        let intensity = if at.elevation <= 0.0 {
            0.0
        } else {
            at.elevation.to_radians().sin().max(0.0).powf(0.6)
                / reference.elevation.to_radians().sin().powf(0.6)
                * smooth(0.0, 3.0, at.elevation)
        };
        let ambient = ambient_raw(at.elevation) / ambient_raw(reference.elevation);
        let (lamp, energy, sky) = lamp(at.elevation);
        Light {
            colour,
            intensity,
            ambient,
            lamp,
            energy,
            sky,
        }
    }

    pub fn authored(&self, authored: &Authored, reference_hour: f64) -> AuthoredLight {
        let at = self.sun();
        let reference = reference_place(reference_hour).sun();
        let authored_toward = unit(authored.toward);
        let authored_elevation = authored_toward[1].clamp(-1.0, 1.0).asin();
        let horizontal = (-authored_toward[0]).atan2(authored_toward[2]);
        let turn = (at.azimuth - reference.azimuth).to_radians()
            - (self.heading - Self::HEADING).to_radians();
        let lifted = (authored_elevation * at.elevation.to_radians()
            / reference.elevation.to_radians().max(1e-3))
        .max(0.5_f64.to_radians());
        let across = horizontal + turn;
        let toward = [
            -across.sin() * lifted.cos(),
            lifted.sin(),
            across.cos() * lifted.cos(),
        ];
        let tint = kelvin(temperature(at.elevation));
        let base = kelvin(temperature(reference.elevation));
        let mut colour = [0, 1, 2]
            .map(|channel| authored.colour[channel] * tint[channel] / base[channel].max(1e-3));
        let peak = colour.into_iter().fold(0.0_f64, f64::max).max(1e-6);
        colour = colour.map(|channel| channel / peak);
        let light = self.light(reference_hour);
        let turned = (toward[0] * authored_toward[0]
            + toward[1] * authored_toward[1]
            + toward[2] * authored_toward[2])
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        let cool = smooth(30.0, 55.0, turned) * smooth(0.0, 20.0, at.elevation);
        AuthoredLight {
            toward,
            colour,
            intensity: light.intensity,
            ambient: light.ambient * (1.0 + authored.sky_fill * cool),
            cool,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    pub elevation: f64,
    pub azimuth: f64,
    pub y_up: [f64; 3],
    pub z_up: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub colour: [f64; 3],
    pub intensity: f64,
    pub ambient: f64,
    pub lamp: [f64; 3],
    pub energy: f64,
    pub sky: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Authored {
    pub toward: [f64; 3],
    pub colour: [f64; 3],
    pub sky_fill: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthoredLight {
    pub toward: [f64; 3],
    pub colour: [f64; 3],
    pub intensity: f64,
    pub ambient: f64,
    pub cool: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    pub hour: f64,
    pub index: usize,
}

pub fn probe_weights(hour: f64, probes: &[Probe], count: usize) -> Vec<f64> {
    let mut weights = vec![0.0; count];
    if probes.is_empty() || count == 0 {
        return weights;
    }
    let mut keys: Vec<(f64, usize)> = probes
        .iter()
        .map(|probe| (probe.hour, probe.index))
        .collect();
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    let hour = hour.rem_euclid(24.0);
    let next = keys.iter().position(|(key, _)| *key > hour).unwrap_or(0);
    let prev = (next + keys.len() - 1) % keys.len();
    let (from_hour, from) = keys[prev];
    let (to_hour, to) = keys[next];
    let span = (to_hour - from_hour).rem_euclid(24.0).max(1e-3);
    let t = ((hour - from_hour).rem_euclid(24.0) / span).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    if from == to {
        if from < count {
            weights[from] = 1.0;
        }
    } else {
        if from < count {
            weights[from] = 1.0 - t;
        }
        if to < count {
            weights[to] = t;
        }
    }
    weights
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunArc {
    pub start: f64,
    pub span: f64,
    pub elevation: f64,
    pub azimuth: f64,
    pub sweep: f64,
    pub reference: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DampedClock {
    pub sun: f64,
    pub tint: f64,
    pub elevation: f64,
    pub azimuth: f64,
    pub skywarm: f64,
    pub lamp: f64,
    pub exposure: f64,
    pub warmth: f64,
    pub turn: f64,
    pub arc: SunArc,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DampedSun {
    pub toward: [f64; 3],
    pub colour: [f64; 3],
    pub shade: [f64; 3],
    pub ambient: [f64; 3],
    pub lamp: f64,
    pub exposure: f64,
    pub warmth: f64,
    pub turn: f64,
}

impl DampedClock {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sun: f64,
        tint: f64,
        elevation: f64,
        azimuth: f64,
        skywarm: f64,
        lamp: f64,
        exposure: f64,
        warmth: f64,
        turn: f64,
        arc: SunArc,
    ) -> Self {
        Self {
            sun,
            tint,
            elevation,
            azimuth,
            skywarm,
            lamp,
            exposure,
            warmth,
            turn,
            arc,
        }
    }

    pub fn at(&self, hour: f64, base: [f64; 3]) -> DampedSun {
        let base = unit(base);
        let (elevation, azimuth) = sun_arc(&self.arc, hour.rem_euclid(24.0));
        let (origin_elevation, origin_azimuth) = sun_arc(&self.arc, self.arc.reference);
        let d_elevation = (0.35 * (elevation.max(-10.0) - origin_elevation) + 0.5 * self.elevation)
            .clamp(-24.0, 20.0)
            .to_radians();
        let d_azimuth = (0.3 * (azimuth - origin_azimuth) + 0.5 * self.azimuth).to_radians();
        let el = base[1].clamp(-1.0, 1.0).asin() + d_elevation;
        let az = base[2].atan2(base[0]) + d_azimuth;
        let toward = [el.cos() * az.cos(), el.sin(), el.cos() * az.sin()];
        let sun = smooth(-2.0, 10.0, elevation);
        let low = 1.0 - smooth(4.0, 30.0, elevation);
        let tint = self.tint + 1.4 * low;
        let warm = [1.0, 1.0 - 0.10 * tint, (1.0 - 0.22 * tint).max(0.2)];
        let night = 1.0 - smooth(-8.0, 5.0, elevation);
        let moon = [0.55 * 0.12, 0.65 * 0.12, 0.12];
        let colour = [0, 1, 2]
            .map(|channel| warm[channel] * self.sun * sun + moon[channel] * self.sun * night);
        let room = 0.14 + 0.86 * smooth(-8.0, 15.0, elevation);
        let dusk = low * (1.0 - night);
        let sky = [
            1.0,
            1.0 - 0.07 * self.skywarm,
            (1.0 - 0.16 * self.skywarm).max(0.2),
        ];
        let day = [1.0, 0.86, 0.72];
        let blue = [0.55, 0.65, 1.0];
        let shade = [0, 1, 2].map(|channel| {
            sky[channel]
                * room
                * (day[channel] * dusk + blue[channel] * night + (1.0 - dusk - night).max(0.0))
        });
        let sun_tint = [1.0, 1.0 - 0.10 * self.tint, 1.0 - 0.22 * self.tint];
        let ambient = [0, 1, 2].map(|channel| {
            0.55 * shade[channel]
                + 0.45 * colour[channel] / (self.sun * sun_tint[channel]).max(1e-3)
        });
        DampedSun {
            toward,
            colour,
            shade,
            ambient,
            lamp: self.lamp * (1.0 - smooth(-6.0, 6.0, elevation)),
            exposure: self.exposure * (1.0 + 0.9 * night),
            warmth: self.warmth * (1.0 - night),
            turn: self.turn,
        }
    }
}

fn reference_place(hour: f64) -> Daylight {
    Daylight {
        hour,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    }
}

fn ambient_raw(elevation: f64) -> f64 {
    0.06 + 0.19 * smooth(-12.0, 0.0, elevation) + 0.75 * smooth(0.0, 20.0, elevation)
}

fn lamp(elevation: f64) -> ([f64; 3], f64, f64) {
    let rise = elevation.to_radians().sin().max(0.0);
    let warm = (elevation / 30.0).clamp(0.0, 1.0);
    let low = [1.0, 0.42, 0.18];
    let high = [1.0, 0.97, 0.92];
    let colour = [0, 1, 2].map(|channel| low[channel] + (high[channel] - low[channel]) * warm);
    let energy = 2.0 * rise.sqrt();
    let sky = 0.15 + 0.85 * (rise * 2.0).min(1.0);
    (colour, energy, sky)
}

fn temperature(elevation: f64) -> f64 {
    2000.0 + 3600.0 * smooth(0.0, 40.0, elevation).powf(0.7)
}

fn kelvin(k: f64) -> [f64; 3] {
    let t = k / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_73 * (t - 60.0).powf(-0.133_204_76)
    };
    let g = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_16 * (t - 60.0).powf(-0.075_514_85)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    let rgb = [r, g, b].map(|channel| (channel / 255.0).clamp(0.0, 1.0));
    let peak = rgb.into_iter().fold(0.0_f64, f64::max).max(1e-6);
    rgb.map(|channel| channel / peak)
}

fn smooth(low: f64, high: f64, x: f64) -> f64 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / length, v[1] / length, v[2] / length]
}

fn sun_arc(arc: &SunArc, hour: f64) -> (f64, f64) {
    let f = (hour - arc.start) / arc.span;
    (
        arc.elevation * (std::f64::consts::PI * f).sin(),
        arc.azimuth + arc.sweep * f,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(value: f64, expect: f64) {
        assert!((value - expect).abs() < 1e-4, "{value} is not {expect}");
    }

    fn at(hour: f64, day: f64, latitude: f64, heading: f64) -> Daylight {
        Daylight {
            hour,
            day,
            latitude,
            heading,
        }
    }

    fn num(key: &str, value: f64) -> (String, Reading) {
        (key.to_string(), Reading::Number(value))
    }

    fn fixture() -> Authored {
        Authored {
            toward: [0.2, 0.8, 0.4],
            colour: [1.0, 0.7, 0.4],
            sky_fill: 1.0,
        }
    }

    fn arc() -> SunArc {
        SunArc {
            start: 6.0,
            span: 12.0,
            elevation: 50.0,
            azimuth: -90.0,
            sweep: 180.0,
            reference: 12.0,
        }
    }

    fn clock() -> DampedClock {
        DampedClock::new(2.0, 0.5, 0.0, 0.0, 0.0, 3.0, 1.0, 0.4, 0.2, arc())
    }

    fn probes() -> Vec<Probe> {
        vec![
            Probe {
                hour: 23.0,
                index: 1,
            },
            Probe {
                hour: 5.0,
                index: 1,
            },
            Probe {
                hour: 6.5,
                index: 2,
            },
            Probe {
                hour: 9.0,
                index: 3,
            },
            Probe {
                hour: 13.0,
                index: 0,
            },
            Probe {
                hour: 17.5,
                index: 4,
            },
            Probe {
                hour: 19.5,
                index: 5,
            },
        ]
    }

    #[test]
    fn parse_fills_the_defaults_and_refuses_what_the_harness_refuses() {
        let lit = Daylight::parse([num("hour", 16.5)]).unwrap();
        assert_eq!(
            (lit.hour, lit.day, lit.latitude, lit.heading),
            (16.5, 172.0, 45.0, 180.0)
        );
        let ordered = Daylight::parse([
            num("heading", 90.0),
            num("hour", 8.0),
            num("latitude", -33.9),
            num("day", 100.0),
        ])
        .unwrap();
        assert_eq!(
            (ordered.hour, ordered.day, ordered.latitude, ordered.heading),
            (8.0, 100.0, -33.9, 90.0)
        );
        for (hour, day, latitude, heading) in [(0.0, 1.0, -90.0, 0.0), (24.0, 365.0, 90.0, 360.0)] {
            Daylight::parse([
                num("hour", hour),
                num("day", day),
                num("latitude", latitude),
                num("heading", heading),
            ])
            .unwrap();
        }
        assert_eq!(
            Daylight::parse([num("hour", 25.0)]).unwrap_err(),
            "daylight.hour = 25 is outside 0 to 24"
        );
        assert_eq!(
            Daylight::parse([num("hour", 12.0), num("day", 0.0)]).unwrap_err(),
            "daylight.day = 0 is outside 1 to 365"
        );
        assert_eq!(
            Daylight::parse([num("hour", 12.0), num("day", 366.0)]).unwrap_err(),
            "daylight.day = 366 is outside 1 to 365"
        );
        assert_eq!(
            Daylight::parse([num("moon", 1.0)]).unwrap_err(),
            "daylight knows no 'moon'; it takes hour, day, latitude and heading"
        );
        assert_eq!(
            Daylight::parse([num("hour", 25.0), num("moon", 1.0)]).unwrap_err(),
            "daylight knows no 'moon'; it takes hour, day, latitude and heading"
        );
        assert_eq!(
            Daylight::parse([num("day", 100.0)]).unwrap_err(),
            "daylight needs an hour, 0 to 24"
        );
        assert_eq!(
            Daylight::parse(Vec::<(String, Reading)>::new()).unwrap_err(),
            "daylight needs an hour, 0 to 24"
        );
        assert_eq!(
            Daylight::parse([("hour", Reading::Other("\"noon\"".into()))]).unwrap_err(),
            "daylight.hour is \"noon\", not a number"
        );
        assert_eq!(
            Daylight::parse([
                ("hour", Reading::Other("true".into())),
                ("latitude", Reading::Number(100.0)),
            ])
            .unwrap_err(),
            "daylight.hour is true, not a number"
        );
        assert_eq!(
            Daylight::parse([("latitude", Reading::Other("true".into()))]).unwrap_err(),
            "daylight needs an hour, 0 to 24"
        );
        assert_eq!(
            Daylight::parse([num("hour", 12.0), num("latitude", 90.5)]).unwrap_err(),
            "daylight.latitude = 90.5 is outside -90 to 90"
        );
        assert_eq!(
            Daylight::parse([num("hour", 12.0), num("heading", 361.0)]).unwrap_err(),
            "daylight.heading = 361 is outside 0 to 360"
        );
    }

    #[test]
    fn the_sun_follows_the_axial_tilt() {
        let samples = [
            (12.0, 172.0, 45.0, 68.439_783, 180.0),
            (12.0, 80.0, 0.0, 89.596_52, 180.0),
            (6.0, 80.0, 45.0, -0.285_30, 90.285_31),
            (9.0, 172.0, 45.0, 47.732_76, 105.298_64),
            (15.0, 172.0, 45.0, 47.732_76, 254.701_36),
            (16.0, 172.0, 45.0, 37.275_93, 266.895_01),
            (8.0, 172.0, 45.0, 37.275_93, 93.104_99),
            (0.0, 172.0, 45.0, -21.560_22, 0.0),
            (12.0, 355.0, 45.0, 21.560_22, 180.0),
            (12.0, 172.0, -33.9, 32.660_22, 0.0),
            (18.0, 172.0, 60.0, 20.150_79, 282.231_41),
            (16.5, 172.0, 45.0, 31.974_68, 272.230_07),
        ];
        for (hour, day, latitude, elevation, azimuth) in samples {
            let sun = at(hour, day, latitude, 180.0).sun();
            near(sun.elevation, elevation);
            near(sun.azimuth, azimuth);
            let len =
                (sun.z_up[0] * sun.z_up[0] + sun.z_up[1] * sun.z_up[1] + sun.z_up[2] * sun.z_up[2])
                    .sqrt();
            near(len, 1.0);
            near(sun.y_up[0], sun.z_up[0]);
            near(sun.y_up[1], sun.z_up[2]);
            near(sun.y_up[2], -sun.z_up[1]);
            near(sun.y_up[1], sun.elevation.to_radians().sin());
        }

        let noon = at(12.0, 172.0, 45.0, 180.0).sun();
        assert!((noon.elevation - (90.0 - 45.0 + 23.44)).abs() < 0.2);
        assert!(noon.z_up[0].abs() < 1e-9);
        assert!(noon.z_up[1] < 0.0 && noon.z_up[2] > 0.9);
        assert!(noon.y_up[1] > 0.9 && noon.y_up[2] > 0.0);

        let morning = at(8.0, 172.0, 45.0, 180.0).sun();
        assert!(morning.z_up[0] > 0.3);
        assert!(morning.elevation > 0.0 && morning.elevation < 45.0);
        assert!(morning.y_up[0] > 0.3);

        assert!(at(0.0, 172.0, 45.0, 180.0).sun().elevation < 0.0);
        let winter = at(12.0, 355.0, 45.0, 180.0).sun();
        assert!((winter.elevation - (90.0 - 45.0 - 23.44)).abs() < 0.3);
        assert!(at(12.0, 172.0, 45.0, 0.0).sun().z_up[1] > 0.0);

        let june = at(12.0, 172.0, 45.0, 180.0).sun();
        assert!((june.elevation - 68.4).abs() < 1.0);
        assert!(at(12.0, 80.0, 0.0, 180.0).sun().elevation > 88.0);
        assert!(at(6.0, 80.0, 45.0, 180.0).sun().elevation.abs() < 2.0);
        assert!(at(9.0, 172.0, 45.0, 180.0).sun().azimuth < 180.0);
        assert!(at(15.0, 172.0, 45.0, 180.0).sun().azimuth > 180.0);
    }

    #[test]
    fn a_noon_sun_is_higher_in_summer_than_in_winter() {
        let summer = at(12.0, 172.0, 45.0, 180.0).sun();
        let winter = at(12.0, 355.0, 45.0, 180.0).sun();
        assert!(summer.elevation > winter.elevation + 40.0);
        assert!(summer.y_up[1] > winter.y_up[1]);
        let january = at(12.0, 1.0, 45.0, 180.0).sun();
        assert!(summer.elevation > january.elevation);
    }

    #[test]
    fn light_is_one_at_its_reference_hour_and_authored_light_follows_the_sun() {
        let noon = at(
            REFERENCE_HOUR,
            Daylight::DAY,
            Daylight::LATITUDE,
            Daylight::HEADING,
        );
        let light = noon.light(REFERENCE_HOUR);
        near(light.intensity, 1.0);
        near(light.ambient, 1.0);
        for (got, expect) in light.lamp.into_iter().zip([1.0, 0.97, 0.92]) {
            near(got, expect);
        }
        assert!(light.energy > 1.0);
        near(light.sky, 1.0);

        let sky = noon.authored(&fixture(), REFERENCE_HOUR);
        let toward = unit(fixture().toward);
        for (got, expect) in sky.toward.into_iter().zip(toward) {
            near(got, expect);
        }
        for (got, expect) in sky.colour.into_iter().zip(fixture().colour) {
            near(got, expect);
        }
        near(sky.intensity, 1.0);
        near(sky.ambient, 1.0);
        assert_eq!(sky.cool, 0.0);

        let morning = at(8.0, Daylight::DAY, Daylight::LATITUDE, Daylight::HEADING);
        let moved = morning.authored(&fixture(), REFERENCE_HOUR);
        assert!((moved.toward[0] - sky.toward[0]).abs() > 0.2);
        assert!(moved.intensity < sky.intensity);

        let mut filled = fixture();
        filled.sky_fill = 0.0;
        let mut wide = fixture();
        wide.sky_fill = 2.0;
        let plain = morning.authored(&filled, REFERENCE_HOUR);
        let boosted = morning.authored(&wide, REFERENCE_HOUR);
        assert!(boosted.cool > 0.0);
        near(
            boosted.ambient - plain.ambient,
            plain.ambient * boosted.cool * 2.0,
        );
        assert!(boosted.ambient > plain.ambient);

        let dawn = at(6.0, 80.0, 45.0, 180.0);
        assert!(dawn.sun().elevation.abs() < 2.0);
        let around = dawn.light(REFERENCE_HOUR);
        assert_eq!(around.intensity, 0.0);
        assert_eq!(around.energy, 0.0);
        assert!(around.lamp[2] < 0.3);
        near(around.sky, 0.15);

        for hour in [0.0, 23.0] {
            let place = at(hour, Daylight::DAY, Daylight::LATITUDE, Daylight::HEADING);
            let lit = place.light(REFERENCE_HOUR);
            assert!(place.sun().elevation < 0.0);
            assert_eq!(lit.intensity, 0.0);
            assert_eq!(lit.energy, 0.0);
            assert_eq!(lit.lamp, [1.0, 0.42, 0.18]);
            near(lit.sky, 0.15);
            let sky = place.authored(&fixture(), REFERENCE_HOUR);
            assert_eq!(sky.intensity, 0.0);
            assert!(sky.ambient < 0.2);
        }
    }

    #[test]
    fn probe_weights_sum_to_one_and_wrap_at_midnight() {
        let probes = probes();
        for hour in [
            0.0, 4.0, 5.0, 7.25, 11.0, 13.0, 18.5, 22.0, 23.0, 24.0, -1.0,
        ] {
            let weights = probe_weights(hour, &probes, 6);
            assert_eq!(weights.len(), 6);
            near(weights.iter().sum::<f64>(), 1.0);
            assert!(weights.iter().filter(|weight| **weight > 0.0).count() <= 2);
        }
        let noon = probe_weights(13.0, &probes, 6);
        assert_eq!(noon[0], 1.0);
        let between = probe_weights(11.0, &probes, 6);
        near(between[3], 0.5);
        near(between[0], 0.5);
        assert_eq!(between[1], 0.0);
        assert_eq!(between[2], 0.0);
        assert_eq!(between[4], 0.0);
        assert_eq!(between[5], 0.0);

        let midnight = probe_weights(0.0, &probes, 6);
        assert_eq!(midnight[1], 1.0);
        assert_eq!(probe_weights(24.0, &probes, 6), midnight);
        assert_eq!(probe_weights(4.0, &probes, 6)[1], 1.0);

        let blend = probe_weights(22.0, &probes, 6);
        near(blend[5], 1.0 - 275.0 / 343.0);
        near(blend[1], 275.0 / 343.0);

        let wrap = [
            Probe {
                hour: 22.0,
                index: 0,
            },
            Probe {
                hour: 2.0,
                index: 1,
            },
        ];
        let across = probe_weights(0.0, &wrap, 2);
        near(across[0], 0.5);
        near(across[1], 0.5);
        let before = probe_weights(23.0, &wrap, 2);
        near(before[0], 0.843_75);
        near(before[1], 0.156_25);
        assert_eq!(probe_weights(22.0, &wrap, 2)[0], 1.0);
        assert_eq!(probe_weights(2.0, &wrap, 2)[1], 1.0);
        assert!(
            probe_weights(0.0, &[], 6)
                .iter()
                .all(|weight| *weight == 0.0)
        );
    }

    #[test]
    fn a_damped_clock_holds_its_base_at_the_reference_and_wraps_the_day() {
        let base = [0.3, 0.7, 0.4];
        let clock = clock();
        let noon = clock.at(arc().reference, base);
        let norm = unit(base);
        for (got, expect) in noon.toward.into_iter().zip(norm) {
            near(got, expect);
        }
        assert_eq!(noon.turn, 0.2);
        near(noon.exposure, 1.0);
        near(noon.warmth, 0.4);
        assert_eq!(noon.lamp, 0.0);
        assert!(noon.colour[0] > noon.colour[2]);

        let later = clock.at(15.0, base);
        let rise = later.toward[1].asin().to_degrees() - noon.toward[1].asin().to_degrees();
        let full = {
            let height = |hour: f64| {
                let f = (hour - arc().start) / arc().span;
                arc().elevation * (std::f64::consts::PI * f).sin()
            };
            height(15.0) - height(arc().reference)
        };
        assert!(rise.abs() < full.abs() * 0.5);
        assert!(rise * full > 0.0);

        let midnight = clock.at(0.0, base);
        let same = clock.at(24.0, base);
        for (got, expect) in midnight.toward.into_iter().zip(same.toward) {
            near(got, expect);
        }
        for (got, expect) in midnight.colour.into_iter().zip(same.colour) {
            near(got, expect);
        }
        assert!(midnight.toward[1] < noon.toward[1]);
        assert!(midnight.exposure > noon.exposure);
        assert_eq!(midnight.warmth, 0.0);
        assert!(midnight.lamp > 0.0);
    }
}
