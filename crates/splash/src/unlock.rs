use crate::licence::{Claims, Group, KeyError, civil_day, date, verify};

const APP: &str = "pfx";
const LAST_DAY: &str = "9999-12-31";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tier {
    #[default]
    Free,
    Indie,
    Pro,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Free => "free",
            Tier::Indie => "indie",
            Tier::Pro => "pro",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tier::Free => "Free",
            Tier::Indie => "Indie",
            Tier::Pro => "Pro",
        }
    }

    fn of(tier: &str) -> Option<Tier> {
        match tier {
            "indie" => Some(Tier::Indie),
            "pro" => Some(Tier::Pro),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Licence {
    pub tier: Tier,
    pub licensee: String,
    pub paid_through: String,
    pub grace_days: u32,
    pub covered_through: String,
    pub id: String,
    pub group: Group,
}

impl Licence {
    pub fn may_hide_splash(&self) -> bool {
        self.group.may_hide_splash(self.tier.name())
    }

    pub fn badge(&self) -> Badge {
        match self.group {
            Group::Owner => Badge::Owner,
            Group::FriendsFamily => Badge::FriendsFamily,
            Group::Customer | Group::Education | Group::Nonprofit | Group::Other => match self.tier
            {
                Tier::Free => Badge::Free,
                Tier::Indie => Badge::Indie,
                Tier::Pro => Badge::Pro,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Badge {
    #[default]
    Free,
    Indie,
    Pro,
    Owner,
    FriendsFamily,
}

impl Badge {
    pub const ALL: [Badge; 5] = [
        Badge::Free,
        Badge::Indie,
        Badge::Pro,
        Badge::Owner,
        Badge::FriendsFamily,
    ];

    pub fn of(licence: Option<&Licence>) -> Badge {
        licence.map_or(Badge::Free, Licence::badge)
    }

    pub fn name(self) -> &'static str {
        match self {
            Badge::Free => "free",
            Badge::Indie => "indie",
            Badge::Pro => "pro",
            Badge::Owner => "owner",
            Badge::FriendsFamily => "friends-family",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Badge::Free => "Free",
            Badge::Indie => "Indie",
            Badge::Pro => "Pro",
            Badge::Owner => "Owner",
            Badge::FriendsFamily => "F&F",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    Key(KeyError),
    App(String),
    Tier(String),
    Lifetime,
    Lapsed {
        covered_through: String,
        built: String,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Empty => write!(f, "no key was given"),
            Refusal::Key(KeyError::Shape) => {
                write!(
                    f,
                    "this is not a pfx key: a key is three parts joined by dots"
                )
            }
            Refusal::Key(KeyError::Prefix) => {
                write!(f, "this is not a pfx key: a key starts with PITO1")
            }
            Refusal::Key(KeyError::PublicKey) => {
                write!(f, "this pfx has no valid public key to check keys with")
            }
            Refusal::Key(KeyError::SignatureEncoding | KeyError::Signature) => write!(
                f,
                "the key's signature does not match: it was changed, or not issued for pfx"
            ),
            Refusal::Key(KeyError::Payload) => write!(f, "the key's contents can't be read"),
            Refusal::Key(KeyError::Version) => {
                write!(f, "the key is of a format this pfx doesn't read")
            }
            Refusal::App(app) => write!(f, "this key is for {app}, not pfx"),
            Refusal::Tier(tier) => write!(f, "this key's tier, {tier}, is not a pfx tier"),
            Refusal::Lifetime => write!(
                f,
                "this key has no paid-through date; pfx takes timed keys only"
            ),
            Refusal::Lapsed {
                covered_through,
                built,
            } => write!(
                f,
                "this key covers builds through {covered_through}, and this pfx was built {built}"
            ),
        }
    }
}

pub fn unlock(public_key: &str, key: &str, build_date: Option<&str>) -> Result<Licence, Refusal> {
    if key.trim().is_empty() {
        return Err(Refusal::Empty);
    }
    let claims = verify(public_key, key).map_err(Refusal::Key)?;
    licence(&claims, build_date)
}

pub fn licence(claims: &Claims, build_date: Option<&str>) -> Result<Licence, Refusal> {
    if claims.app != APP {
        return Err(Refusal::App(claims.app.clone()));
    }
    let tier = Tier::of(&claims.tier).ok_or_else(|| Refusal::Tier(claims.tier.clone()))?;
    let (true, Some(paid_through)) = (claims.is_timed(), claims.paid_through.clone()) else {
        return Err(Refusal::Lifetime);
    };
    let grace_days = claims.grace_days.unwrap_or(0);
    let covered_through = civil_day(&paid_through)
        .map(|day| {
            date((day + i64::from(grace_days)) * 86_400).unwrap_or_else(|| LAST_DAY.to_string())
        })
        .ok_or(Refusal::Key(KeyError::Payload))?;
    if let Some(built) = build_date.filter(|built| civil_day(built).is_some())
        && !claims.covers_build(built)
    {
        return Err(Refusal::Lapsed {
            covered_through,
            built: built.to_string(),
        });
    }
    Ok(Licence {
        tier,
        licensee: claims
            .licensee
            .clone()
            .filter(|licensee| !licensee.trim().is_empty())
            .unwrap_or_else(|| claims.email.clone()),
        paid_through,
        grace_days,
        covered_through,
        id: claims.id.clone(),
        group: claims.group,
    })
}
