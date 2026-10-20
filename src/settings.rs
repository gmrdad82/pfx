use std::io::Read as _;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use pfx_settings::{Effect, Key, LICENCE, Preferences, Store, Value as Setting};
use pfx_splash::Licence;
use serde_json::{Map, Value, json};

#[derive(Parser)]
#[command(name = "pfx", arg_required_else_help = false)]
struct Line {
    #[command(subcommand)]
    verb: Top,
}

#[derive(Subcommand)]
enum Top {
    #[command(
        about = "Read and set pfx's own settings: VRR, the frame rate, the quality, each effect and the licence",
        subcommand_required = true,
        arg_required_else_help = false
    )]
    Settings(Args),
}

#[derive(clap::Args)]
struct Args {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    #[command(about = "Print every setting, or one by its key")]
    Get(Get),
    #[command(about = "Set one setting; the editor and games read it as they start")]
    Set(Set),
    #[command(about = "Check a pfx key offline and keep it in the settings")]
    Unlock(Unlock),
}

#[derive(clap::Args)]
struct Get {
    #[arg(
        value_name = "KEY",
        help = "vrr, frame_rate, stats, quality, effects.<effect> or licence"
    )]
    key: Option<String>,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Set {
    #[arg(
        value_name = "KEY",
        help = "vrr, frame_rate, stats, quality or effects.<effect>"
    )]
    key: String,
    #[arg(
        value_name = "VALUE",
        help = "on or off; 60, 120, uncapped or auto; appearance or performance"
    )]
    value: String,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Unlock {
    #[arg(
        value_name = "KEY_FILE",
        help = "A file holding the key, as issued or with its armour lines, or - to read it from stdin"
    )]
    file: String,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

pub fn claims(args: &[String]) -> bool {
    args.first().is_some_and(|word| word == "settings")
}

#[cfg(test)]
pub fn help_text(args: &[String]) -> Option<String> {
    let words = std::iter::once("pfx").chain(args.iter().map(String::as_str));
    match Line::try_parse_from(words) {
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            Some(error.render().to_string())
        }
        _ => None,
    }
}

pub struct Checker {
    pub public_key: Option<String>,
    pub built: Option<String>,
}

impl Checker {
    fn engine() -> Checker {
        Checker {
            public_key: pfx_splash::PUBLIC_KEY.map(str::to_string),
            built: pfx_splash::built(),
        }
    }

    fn status(&self, key: Option<&str>) -> Option<Licence> {
        pfx_splash::unlock(self.public_key.as_deref()?, key?, self.built.as_deref()).ok()
    }
}

pub fn main(args: &[String]) -> ExitCode {
    let words = std::iter::once("pfx").chain(args.iter().map(String::as_str));
    let Top::Settings(parsed) = match Line::try_parse_from(words) {
        Ok(line) => line.verb,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    let stdin = || {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map(|_| text)
            .map_err(|error| format!("cannot read the key from stdin: {error}"))
    };
    match run(&parsed.verb, &Store::machine(), &Checker::engine(), stdin) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("pfx settings: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    verb: &Verb,
    store: &Store,
    checker: &Checker,
    stdin: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let path = store
        .path()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    match verb {
        Verb::Get(get) => {
            let preferences = store.preferences();
            let licence = checker.status(store.licence_key().as_deref());
            match get.key.as_deref() {
                None if get.json => Ok(json!({
                    "path": path,
                    "measured": preferences.measured(),
                    "settings": every(&preferences, licence.as_ref()),
                })
                .to_string()),
                None => Ok(lines(&preferences, licence.as_ref())),
                Some(LICENCE) if get.json => Ok(licence_json(licence.as_ref()).to_string()),
                Some(LICENCE) => Ok(licence_line(licence.as_ref())),
                Some(name) => {
                    let key = Key::parse(name)?;
                    let shown = key.get(&preferences);
                    Ok(if get.json {
                        json!({"key": key.name(), "value": value(&shown)}).to_string()
                    } else {
                        shown.to_string()
                    })
                }
            }
        }
        Verb::Set(set) => {
            if set.key == LICENCE {
                return Err(
                    "the licence is set by checking a key: pfx settings unlock <KEY_FILE|->".into(),
                );
            }
            let key = Key::parse(&set.key)?;
            let mut preferences = store.preferences();
            key.set(&mut preferences, &set.value)?;
            store
                .keep_preferences(&preferences)
                .map_err(|error| format!("cannot write {path}: {error}"))?;
            let shown = key.get(&preferences);
            Ok(if set.json {
                json!({"key": key.name(), "value": value(&shown), "path": path}).to_string()
            } else {
                format!("{} = {shown}", key.name())
            })
        }
        Verb::Unlock(unlock) => {
            let text = if unlock.file == "-" {
                stdin()?
            } else {
                std::fs::read_to_string(&unlock.file)
                    .map_err(|error| format!("cannot read {}: {error}", unlock.file))?
            };
            let public_key = checker
                .public_key
                .as_deref()
                .ok_or("this pfx was built without a public key to check keys with")?;
            let licence = pfx_splash::unlock(public_key, &text, checker.built.as_deref())
                .map_err(|refusal| refusal.to_string())?;
            store
                .keep_licence_key(Some(&pfx_splash::token(&text)))
                .map_err(|error| format!("cannot write {path}: {error}"))?;
            Ok(if unlock.json {
                let mut shown = licence_json(Some(&licence));
                shown["path"] = json!(path);
                shown.to_string()
            } else {
                format!("{}; kept in {path}", licence_line(Some(&licence)))
            })
        }
    }
}

fn value(shown: &Setting) -> Value {
    match shown {
        Setting::Bool(on) => json!(on),
        Setting::Text(text) => json!(text),
    }
}

fn every(preferences: &Preferences, licence: Option<&Licence>) -> Value {
    let mut effects = Map::new();
    for effect in Effect::ALL {
        effects.insert(
            effect.name().into(),
            json!(preferences.graphics.effects.get(effect)),
        );
    }
    json!({
        "vrr": preferences.vrr,
        "frame_rate": preferences.rate().name(),
        "quality": preferences.graphics.quality().name(),
        "effects": effects,
        "licence": licence_json(licence),
    })
}

fn lines(preferences: &Preferences, licence: Option<&Licence>) -> String {
    let mut lines: Vec<String> = Key::all()
        .into_iter()
        .map(|key| format!("{} = {}", key.name(), key.get(preferences)))
        .collect();
    lines.push(format!("{LICENCE} = {}", licence_line(licence)));
    lines.join("\n")
}

fn licence_line(licence: Option<&Licence>) -> String {
    match licence {
        Some(licence) => format!(
            "pfx {} · {} · covers builds through {}",
            licence.tier.label(),
            licence.licensee,
            licence.covered_through
        ),
        None => "pfx Free".into(),
    }
}

fn licence_json(licence: Option<&Licence>) -> Value {
    match licence {
        Some(licence) => json!({
            "tier": licence.tier.name(),
            "licensee": licence.licensee,
            "paid_through": licence.paid_through,
            "grace_days": licence.grace_days,
            "covered_through": licence.covered_through,
            "id": licence.id,
        }),
        None => json!({"tier": "free"}),
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn parsed(line: &str) -> Result<Verb, String> {
        let words = std::iter::once("pfx".to_string()).chain(words(line));
        Line::try_parse_from(words)
            .map(|line| {
                let Top::Settings(args) = line.verb;
                args.verb
            })
            .map_err(|error| error.render().to_string())
    }

    fn store(name: &str) -> Store {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp/settings-cli")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Store::in_dir(&dir)
    }

    fn checker() -> (SigningKey, Checker) {
        let signing = SigningKey::from_bytes(&[3u8; 32]);
        let checker = Checker {
            public_key: Some(STANDARD.encode(signing.verifying_key().to_bytes())),
            built: Some("2026-10-20".into()),
        };
        (signing, checker)
    }

    fn mint(signing: &SigningKey, tier: &str, kind: &str) -> String {
        let payload = format!(
            r#"{{"v":2,"id":"k7","email":"a@example.com","licensee":"Example Games","app":"pfx","tier":"{tier}","major":3,"issued":"2026-10-01"{kind}}}"#
        );
        let signed = format!("PITO1.{}", URL_SAFE_NO_PAD.encode(payload.as_bytes()));
        let signature = signing.sign(signed.as_bytes());
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }

    const TIMED: &str = r#","type":"timed","paid_through":"2026-12-31","grace_days":14"#;

    fn ran(line: &str, store: &Store, checker: &Checker, key: &str) -> Result<String, String> {
        let verb = parsed(line).unwrap();
        let key = key.to_string();
        run(&verb, store, checker, move || Ok(key))
    }

    #[test]
    fn get_and_set_print_their_lines_and_json_and_keep_the_value() {
        let store = store("get-set");
        let (_, checker) = checker();
        assert_eq!(
            ran("settings set vrr off", &store, &checker, "").unwrap(),
            "vrr = off"
        );
        assert_eq!(
            ran("settings set frame_rate auto --json", &store, &checker, "").unwrap(),
            json!({"key": "frame_rate", "value": "auto", "path": store.path().unwrap().display().to_string()}).to_string()
        );
        ran("settings set effects.ssr off", &store, &checker, "").unwrap();
        assert_eq!(
            ran("settings get vrr", &store, &checker, "").unwrap(),
            "off"
        );
        assert_eq!(
            ran("settings get effects.ssr --json", &store, &checker, "").unwrap(),
            r#"{"key":"effects.ssr","value":false}"#
        );
        let all = ran("settings get", &store, &checker, "").unwrap();
        assert!(all.starts_with("vrr = off\nframe_rate = auto\nstats = off\nquality = appearance\neffects.taa = on\neffects.ssr = off\n"), "{all}");
        assert!(all.ends_with("licence = pfx Free"), "{all}");
        let json: Value =
            serde_json::from_str(&ran("settings get --json", &store, &checker, "").unwrap())
                .unwrap();
        assert_eq!(json["settings"]["effects"]["ssr"], json!(false));
        assert_eq!(json["settings"]["licence"]["tier"], json!("free"));
        assert_eq!(json["measured"], json!(false));
    }

    #[test]
    fn set_refuses_unknown_keys_bad_values_and_the_licence() {
        let store = store("refused");
        let (_, checker) = checker();
        let fail = |line: &str| ran(line, &store, &checker, "").unwrap_err();
        assert!(fail("settings set chrome on").starts_with("no setting is named 'chrome'"));
        assert_eq!(
            fail("settings set quality ultra"),
            "'ultra' is not a value of quality; it takes appearance, performance"
        );
        assert!(fail("settings set licence pro").contains("pfx settings unlock"));
        assert!(fail("settings get effects.chrome").starts_with("no setting is named"));
        assert!(fail("settings unlock tmp/settings-cli/none.key").starts_with("cannot read"));
        assert_eq!(store.preferences(), Preferences::default());
    }

    #[test]
    fn unlock_keeps_a_verified_timed_key_and_refuses_the_rest_plainly() {
        let store = store("unlock");
        let (signing, checker) = checker();
        let pro = mint(&signing, "pro", TIMED);
        let said = ran("settings unlock -", &store, &checker, &format!("{pro}\n")).unwrap();
        assert!(
            said.starts_with(
                "pfx Pro · Example Games · covers builds through 2027-01-14; kept in "
            ),
            "{said}"
        );
        assert_eq!(store.licence_key().as_deref(), Some(pro.as_str()));
        let shown: Value = serde_json::from_str(
            &ran("settings get licence --json", &store, &checker, "").unwrap(),
        )
        .unwrap();
        assert_eq!(shown["tier"], json!("pro"));
        assert_eq!(shown["covered_through"], json!("2027-01-14"));
        let other = SigningKey::from_bytes(&[4u8; 32]);
        for (key, why) in [
            (
                mint(&other, "pro", TIMED),
                "the key's signature does not match",
            ),
            (mint(&signing, "pro", ""), "pfx takes timed keys only"),
            (
                mint(
                    &signing,
                    "pro",
                    r#","type":"timed","paid_through":"2026-09-01","grace_days":0"#,
                ),
                "covers builds through 2026-09-01, and this pfx was built 2026-10-20",
            ),
            ("PITO2.a.b".into(), "a key starts with PITO1"),
            (String::new(), "no key was given"),
        ] {
            let refused = ran("settings unlock - --json", &store, &checker, &key).unwrap_err();
            assert!(refused.contains(why), "{refused}");
        }
        assert_eq!(store.licence_key().as_deref(), Some(pro.as_str()));
        let indie = mint(&signing, "indie", TIMED);
        ran("settings unlock -", &store, &checker, &indie).unwrap();
        assert_eq!(
            ran("settings get licence", &store, &checker, "").unwrap(),
            "pfx Indie · Example Games · covers builds through 2027-01-14"
        );
    }

    #[test]
    fn the_verbs_parse_in_clap_s_shape_and_refuse_in_clap_s_words() {
        assert!(matches!(parsed("settings get"), Ok(Verb::Get(_))));
        assert!(matches!(
            parsed("settings set vrr on --json"),
            Ok(Verb::Set(_))
        ));
        assert!(matches!(parsed("settings unlock -"), Ok(Verb::Unlock(_))));
        for (line, said) in [
            (
                "settings",
                "error: 'pfx settings' requires a subcommand but one was not provided",
            ),
            ("settings tune", "error: unrecognized subcommand 'tune'"),
            (
                "settings set vrr",
                "error: the following required arguments were not provided",
            ),
            (
                "settings unlock",
                "error: the following required arguments were not provided",
            ),
            ("settings get --x", "error: unexpected argument '--x' found"),
        ] {
            let refused = parsed(line)
                .err()
                .unwrap_or_else(|| panic!("{line} parsed"));
            assert!(refused.starts_with(said), "{line}:\n{refused}");
            assert!(
                refused.contains("Usage: pfx settings"),
                "{line}:\n{refused}"
            );
            assert!(
                refused
                    .trim_end()
                    .ends_with("For more information, try '--help'."),
                "{line}:\n{refused}"
            );
        }
    }
}
