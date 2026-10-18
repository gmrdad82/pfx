use std::path::{Path, PathBuf};

use pfx_file::{Codec, Reader, Tag, Writer};
use toml::{Table, Value};

use crate::Source;
use crate::licence::{Kind, Policy};
use crate::secret::Credential;

pub const TAG: Tag = *b"CONF";
pub const FILE: &str = "settings.pitofx";
pub const ACCOUNT_KINDS: [&str; 1] = ["sketchfab"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub kind: String,
    pub credential: Credential,
}

impl Account {
    pub fn source(&self) -> Box<dyn Source> {
        Box::new(crate::sketchfab::Sketchfab {
            name: self.name.clone(),
            credential: self.credential.clone(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub policy: Policy,
    pub accounts: Vec<Account>,
}

fn strings(value: &Value, at: &str) -> Result<Vec<String>, String> {
    value
        .as_array()
        .and_then(|list| {
            list.iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| format!("{at}: expected an array of strings"))
}

fn known(table: &Table, at: &str, keys: &[&str]) -> Result<(), String> {
    match table.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(key) if key.contains("token") => Err(format!(
            "{at}.{key}: pfx stores no token; name an environment variable (token_env) or a command (token_cmd) that prints it"
        )),
        Some(key) => Err(format!(
            "{at}.{key}: unknown key, expected one of {}",
            keys.join(", ")
        )),
        None => Ok(()),
    }
}

impl Settings {
    pub fn parse(text: &str) -> Result<Settings, String> {
        let root: Table = toml::from_str(text).map_err(|e| e.to_string())?;
        known(&root, "settings", &["licences", "account"])?;
        let mut out = Settings::default();
        if let Some(licences) = root.get("licences") {
            let table = licences.as_table().ok_or("licences: expected a table")?;
            known(table, "licences", &["allow", "only"])?;
            if let Some(allow) = table.get("allow") {
                for word in strings(allow, "licences.allow")? {
                    out.policy
                        .allow
                        .insert(Kind::parse(&word).map_err(|e| format!("licences.allow: {e}"))?);
                }
            }
            if let Some(only) = table.get("only") {
                for id in strings(only, "licences.only")? {
                    pfx_core::spdx::validate(&id).map_err(|e| format!("licences.only: {e}"))?;
                    out.policy.only.push(id);
                }
            }
        }
        if let Some(accounts) = root.get("account") {
            let list = accounts
                .as_array()
                .ok_or("account: expected an array of tables, [[account]]")?;
            for (index, entry) in list.iter().enumerate() {
                let at = format!("account[{index}]");
                let table = entry
                    .as_table()
                    .ok_or_else(|| format!("{at}: expected a table"))?;
                known(table, &at, &["name", "kind", "token_env", "token_cmd"])?;
                let text = |key: &str| {
                    table
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| format!("{at}.{key}: missing, expected a string"))
                };
                let name = text("name")?;
                crate::check_id(&name).map_err(|e| format!("{at}.name: {e}"))?;
                let kind = text("kind")?;
                if !ACCOUNT_KINDS.contains(&kind.as_str()) {
                    return Err(format!(
                        "{at}.kind: unknown account kind `{kind}`, expected one of: {}",
                        ACCOUNT_KINDS.join(", ")
                    ));
                }
                let credential = match (table.get("token_env"), table.get("token_cmd")) {
                    (Some(var), None) => Credential::Env(
                        var.as_str()
                            .filter(|v| !v.is_empty())
                            .ok_or_else(|| format!("{at}.token_env: expected a variable name"))?
                            .to_string(),
                    ),
                    (None, Some(cmd)) => {
                        let argv = strings(cmd, &format!("{at}.token_cmd"))?;
                        if argv.is_empty() {
                            return Err(format!("{at}.token_cmd: names no command"));
                        }
                        Credential::Command(argv)
                    }
                    (Some(_), Some(_)) => {
                        return Err(format!("{at}: give token_env or token_cmd, not both"));
                    }
                    (None, None) => {
                        return Err(format!("{at}: give token_env or token_cmd"));
                    }
                };
                if crate::builtin().iter().any(|s| s.name() == name)
                    || out.accounts.iter().any(|a| a.name == name)
                {
                    return Err(format!("{at}.name: `{name}` is already a source"));
                }
                out.accounts.push(Account {
                    name,
                    kind,
                    credential,
                });
            }
        }
        Ok(out)
    }

    pub fn text(&self) -> String {
        let mut root = Table::new();
        let mut licences = Table::new();
        licences.insert(
            "allow".into(),
            Value::Array(
                self.policy
                    .allow
                    .iter()
                    .map(|k| Value::String(k.word().into()))
                    .collect(),
            ),
        );
        if !self.policy.only.is_empty() {
            licences.insert(
                "only".into(),
                Value::Array(
                    self.policy
                        .only
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        root.insert("licences".into(), Value::Table(licences));
        let accounts: Vec<Value> = self
            .accounts
            .iter()
            .map(|a| {
                let mut t = Table::new();
                t.insert("name".into(), Value::String(a.name.clone()));
                t.insert("kind".into(), Value::String(a.kind.clone()));
                match &a.credential {
                    Credential::Env(var) => {
                        t.insert("token_env".into(), Value::String(var.clone()));
                    }
                    Credential::Command(argv) => {
                        t.insert(
                            "token_cmd".into(),
                            Value::Array(argv.iter().cloned().map(Value::String).collect()),
                        );
                    }
                }
                Value::Table(t)
            })
            .collect();
        if !accounts.is_empty() {
            root.insert("account".into(), Value::Array(accounts));
        }
        toml::to_string(&root).unwrap_or_default()
    }

    pub fn load(path: &Path) -> Result<Settings, String> {
        let legacy = path.with_extension(pfx_file::LEGACY_EXTENSION);
        if !path.exists() && pfx_file::is_file_path(path) && pfx_file::has_magic(&legacy) {
            return Settings::load(&legacy);
        }
        if !path.exists() {
            return Ok(Settings::default());
        }
        let reader = Reader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = reader
            .chunk(TAG)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .ok_or_else(|| format!("{}: holds no pfx settings", path.display()))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| format!("{}: the settings are not UTF-8", path.display()))?;
        Settings::parse(text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        Writer::new()
            .chunk(TAG, self.text().into_bytes(), Codec::Auto)
            .write(path)
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

pub fn path() -> Result<PathBuf, String> {
    if let Some(file) = var("PFX_SETTINGS") {
        return Ok(file);
    }
    let dir = var("PFX_CONFIG")
        .or_else(|| {
            if cfg!(windows) {
                var("APPDATA").map(|d| d.join("pfx"))
            } else if cfg!(target_os = "macos") {
                var("HOME").map(|h| h.join("Library/Application Support/pfx"))
            } else {
                var("XDG_CONFIG_HOME")
                    .map(|d| d.join("pfx"))
                    .or_else(|| var("HOME").map(|h| h.join(".config/pfx")))
            }
        })
        .ok_or("no settings folder: set PFX_SETTINGS to a .pitofx file")?;
    Ok(dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"
[licences]
allow = ["nc"]
only = ["CC0-1.0", "CC-BY-4.0", "CC-BY-NC-4.0"]

[[account]]
name = "models"
kind = "sketchfab"
token_env = "MODELS_TOKEN"

[[account]]
name = "models-cmd"
kind = "sketchfab"
token_cmd = ["secret-tool", "lookup", "service", "models"]
"#;

    #[test]
    fn settings_round_trip_through_a_pfx_and_hold_no_token() {
        let settings = Settings::parse(TEXT).unwrap();
        assert!(settings.policy.allow.contains(&Kind::NonCommercial));
        assert_eq!(settings.accounts.len(), 2);
        let dir = crate::mock::scratch("settings");
        let file = dir.join(FILE);
        settings.save(&file).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(&bytes[..8], &pfx_file::MAGIC);
        assert_eq!(Settings::load(&file).unwrap(), settings);
        assert_eq!(
            Settings::load(&dir.join("none.pitofx")).unwrap(),
            Settings::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stored_token_or_a_clash_is_refused() {
        let stored =
            Settings::parse("[[account]]\nname = \"m\"\nkind = \"sketchfab\"\ntoken = \"abc\"\n")
                .unwrap_err();
        assert!(stored.contains("stores no token"), "{stored}");
        assert!(!stored.contains("abc"));
        let clash = Settings::parse(
            "[[account]]\nname = \"polyhaven\"\nkind = \"sketchfab\"\ntoken_env = \"T\"\n",
        )
        .unwrap_err();
        assert!(clash.contains("already a source"), "{clash}");
        assert!(Settings::parse("[[account]]\nname = \"m\"\nkind = \"sketchfab\"\n").is_err());
        assert!(
            Settings::parse("[[account]]\nname = \"m\"\nkind = \"ftp\"\ntoken_env = \"T\"\n")
                .is_err()
        );
        assert!(Settings::parse("[licences]\nallow = [\"sa\"]\n").is_err());
        assert!(Settings::parse("[licences]\nonly = [\"Nope-1\"]\n").is_err());
    }
}
