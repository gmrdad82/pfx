use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

use crate::settings::Settings;

pub const PUBLIC_KEY: Option<&str> = Some("jeSuvgBw+ExXNw32a/dbweivyLRLNGyE67aSb8BBfdg=");
const PREFIX: &str = "PITO1";
const MAX_LEN: usize = 16 * 1024;
const VERSION: u32 = 2;
const LIFETIME: &str = "lifetime";
const TIMED: &str = "timed";
const APP: &str = "pfx";
const PRO: &str = "pro";
const BUILT: Option<&str> = option_env!("SOURCE_DATE_EPOCH");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    Shape,
    Prefix,
    PublicKey,
    SignatureEncoding,
    Signature,
    Payload,
    Version,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Claims {
    pub v: u32,
    pub id: String,
    pub email: String,
    pub app: String,
    pub tier: String,
    pub major: u32,
    #[serde(default = "one")]
    pub device_cap: u32,
    pub issued: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub paid_through: Option<String>,
    #[serde(default)]
    pub grace_days: Option<u32>,
    #[serde(default)]
    pub licensee: Option<String>,
}

#[derive(Deserialize)]
struct Version {
    v: u32,
}

fn one() -> u32 {
    1
}

impl Claims {
    pub fn is_timed(&self) -> bool {
        self.kind.as_deref() == Some(TIMED)
    }

    pub fn covers_build(&self, build_date: &str) -> bool {
        if !self.is_timed() {
            return true;
        }
        let (Some(through), Some(built)) = (
            self.paid_through.as_deref().and_then(civil_day),
            civil_day(build_date),
        ) else {
            return false;
        };
        built <= through + i64::from(self.grace_days.unwrap_or(0))
    }
}

pub fn verify(public_key: &str, key: &str) -> Result<Claims, KeyError> {
    let payload = signed(public_key, key)?;
    let version: Version = serde_json::from_slice(&payload).map_err(|_| KeyError::Payload)?;
    if version.v != VERSION {
        return Err(KeyError::Version);
    }
    let claims: Claims = serde_json::from_slice(&payload).map_err(|_| KeyError::Payload)?;
    match claims.kind.as_deref() {
        None | Some(LIFETIME) => Ok(claims),
        Some(TIMED) if claims.paid_through.as_deref().and_then(civil_day).is_some() => Ok(claims),
        _ => Err(KeyError::Payload),
    }
}

pub fn covers(claims: &Claims, build_date: Option<&str>) -> bool {
    claims.app == APP
        && claims.tier == PRO
        && claims.is_timed()
        && build_date.is_some_and(|date| claims.covers_build(date))
}

pub fn hidden(settings: &Settings, public_key: Option<&str>, build_date: Option<&str>) -> bool {
    let (true, Some(key), Some(public_key)) = (settings.hide, settings.key.as_deref(), public_key)
    else {
        return false;
    };
    verify(public_key, key).is_ok_and(|claims| covers(&claims, build_date))
}

pub fn built() -> Option<String> {
    date(BUILT?.trim().parse().ok()?)
}

pub fn build_date(game: Option<&str>) -> Option<String> {
    game.filter(|date| civil_day(date).is_some())
        .map(str::to_string)
        .or_else(built)
}

pub(crate) fn date(epoch: i64) -> Option<String> {
    let days = epoch.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (0..=9999)
        .contains(&year)
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

pub(crate) fn civil_day(date: &str) -> Option<i64> {
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        bytes[range.clone()]
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| date[range].parse().ok())?
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > month_days {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

pub fn token(pasted: &str) -> String {
    normalize(pasted)
}

fn normalize(pasted: &str) -> String {
    pasted
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            let dashes = trimmed.chars().take_while(|c| *c == '-').count();
            dashes == 0
                || !trimmed[dashes..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn signed(public_key: &str, key: &str) -> Result<Vec<u8>, KeyError> {
    if key.len() > MAX_LEN {
        return Err(KeyError::Shape);
    }
    let cleaned = normalize(key);
    let parts: Vec<&str> = cleaned.splitn(4, '.').collect();
    if parts.len() != 3 {
        return Err(KeyError::Shape);
    }
    if parts[0].to_uppercase() != PREFIX {
        return Err(KeyError::Prefix);
    }
    let bytes: [u8; 32] = STANDARD
        .decode(public_key.trim())
        .map_err(|_| KeyError::PublicKey)?
        .try_into()
        .map_err(|_| KeyError::PublicKey)?;
    let verifying = VerifyingKey::from_bytes(&bytes).map_err(|_| KeyError::PublicKey)?;
    let signature: [u8; 64] = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| KeyError::SignatureEncoding)?
        .try_into()
        .map_err(|_| KeyError::SignatureEncoding)?;
    let message = format!("{PREFIX}.{}", parts[1]);
    verifying
        .verify_strict(message.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| KeyError::Signature)?;
    URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| KeyError::Payload)
}
