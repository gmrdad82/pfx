use serde::Serialize;
use serde_json::{Map, Value};

pub const DAYLIGHT_KEYS: &[&str] = &["hour", "day", "latitude", "heading"];
const TILT: f64 = 23.44;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Daylight {
    pub hour: Option<f64>,
    pub day: f64,
    pub latitude: f64,
    pub heading: f64,
}

impl Default for Daylight {
    fn default() -> Self {
        Daylight {
            hour: None,
            day: 172.0,
            latitude: 45.0,
            heading: 180.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sun {
    pub dir: [f64; 3],
    pub elevation: f64,
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    let n = match value {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        other => other.as_f64(),
    };
    n.filter(|n| n.is_finite())
        .ok_or_else(|| format!("daylight.{key} is a number"))
}

impl Daylight {
    pub fn apply(&mut self, table: &Map<String, Value>, place: &str) -> Result<(), String> {
        for (key, value) in table {
            let v = number(value, key).map_err(|e| format!("{place}: {e}"))?;
            match key.as_str() {
                "hour" if (0.0..=24.0).contains(&v) => self.hour = Some(v),
                "day" if (1.0..=366.0).contains(&v) => self.day = v,
                "latitude" if (-90.0..=90.0).contains(&v) => self.latitude = v,
                "heading" => self.heading = v.rem_euclid(360.0),
                "hour" | "day" | "latitude" => {
                    return Err(format!("{place}: daylight.{key} {v} is out of range"));
                }
                other => {
                    return Err(format!(
                        "{place}: unknown daylight key {other}; allowed: {}",
                        DAYLIGHT_KEYS.join(", ")
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn sun(&self, hour: f64) -> Sun {
        let rad = f64::to_radians;
        let declination = rad(TILT * (std::f64::consts::TAU * (284.0 + self.day) / 365.0).sin());
        let angle = rad(15.0 * (hour - 12.0));
        let lat = rad(self.latitude);
        let elevation = (lat.sin() * declination.sin()
            + lat.cos() * declination.cos() * angle.cos())
        .clamp(-1.0, 1.0)
        .asin();
        let bearing = (-declination.cos() * angle.sin())
            .atan2(declination.sin() * lat.cos() - declination.cos() * angle.cos() * lat.sin());
        let flat = elevation.cos();
        let front = rad(self.heading);
        let right = front - rad(90.0);
        Sun {
            dir: [
                flat * (bearing - right).cos(),
                -flat * (bearing - front).cos(),
                elevation.sin(),
            ],
            elevation: elevation.to_degrees(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_in_june_at_45_north_stands_high_in_front_of_a_south_facing_scene() {
        let sun = Daylight::default().sun(12.0);
        assert!((sun.elevation - (90.0 - 45.0 + 23.44)).abs() < 0.2);
        assert!(sun.dir[0].abs() < 1e-9);
        assert!(sun.dir[1] < 0.0 && sun.dir[2] > 0.9);
    }

    #[test]
    fn morning_light_comes_from_the_east_which_is_the_right_of_a_south_facing_scene() {
        let sun = Daylight::default().sun(8.0);
        assert!(sun.dir[0] > 0.3);
        assert!(sun.elevation > 0.0 && sun.elevation < 45.0);
    }

    #[test]
    fn midnight_is_below_the_horizon_and_december_noon_is_low() {
        let d = Daylight::default();
        assert!(d.sun(0.0).elevation < 0.0);
        let winter = Daylight {
            day: 355.0,
            ..Daylight::default()
        };
        assert!((winter.sun(12.0).elevation - (90.0 - 45.0 - 23.44)).abs() < 0.3);
    }

    #[test]
    fn a_north_facing_scene_sees_the_noon_sun_behind_it() {
        let d = Daylight {
            heading: 0.0,
            ..Daylight::default()
        };
        assert!(d.sun(12.0).dir[1] > 0.0);
    }

    #[test]
    fn tables_are_checked() {
        let mut d = Daylight::default();
        let ok: Map<String, Value> =
            serde_json::from_str(r#"{"hour": 17.5, "latitude": "-33.9"}"#).unwrap();
        d.apply(&ok, "shot").unwrap();
        assert_eq!((d.hour, d.latitude), (Some(17.5), -33.9));
        let bad: Map<String, Value> = serde_json::from_str(r#"{"hour": 25}"#).unwrap();
        assert!(d.apply(&bad, "shot").is_err());
        let stray: Map<String, Value> = serde_json::from_str(r#"{"season": 2}"#).unwrap();
        assert!(d.apply(&stray, "shot").is_err());
    }
}
