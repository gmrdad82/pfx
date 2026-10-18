use std::fmt;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    Env(String),
    Command(Vec<String>),
}

pub struct Secret(String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Credential {
    pub fn describe(&self) -> String {
        match self {
            Credential::Env(var) => format!("token_env {var}"),
            Credential::Command(argv) => {
                format!("token_cmd {}", argv.first().map_or("", String::as_str))
            }
        }
    }

    pub fn resolve(&self) -> Result<Secret, String> {
        let token = match self {
            Credential::Env(var) => std::env::var(var)
                .map_err(|_| format!("token_env: {var} is not set or not UTF-8"))?,
            Credential::Command(argv) => {
                let (program, args) = argv.split_first().ok_or("token_cmd: names no command")?;
                let output = Command::new(program)
                    .args(args)
                    .stdin(Stdio::null())
                    .stderr(Stdio::inherit())
                    .output()
                    .map_err(|e| format!("token_cmd: cannot run `{program}`: {e}"))?;
                if !output.status.success() {
                    return Err(format!("token_cmd: `{program}` failed ({})", output.status));
                }
                String::from_utf8(output.stdout)
                    .map_err(|_| format!("token_cmd: `{program}` printed no UTF-8 token"))?
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string()
            }
        };
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err(format!("{}: gave an empty token", self.describe()));
        }
        Ok(Secret(token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_shows_in_debug_output() {
        let secret = Secret("abc123".into());
        assert!(!format!("{secret:?}").contains("abc123"));
    }

    #[test]
    fn a_missing_variable_is_named_and_nothing_else() {
        let error = Credential::Env("PFX_TEST_NO_SUCH_TOKEN_VAR".into())
            .resolve()
            .unwrap_err();
        assert!(error.contains("PFX_TEST_NO_SUCH_TOKEN_VAR"));
        assert!(Credential::Command(Vec::new()).resolve().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_token_command_gives_its_first_line_and_a_failure_hides_its_output() {
        let token = Credential::Command(vec!["printf".into(), "tok-1\\nrest\\n".into()])
            .resolve()
            .unwrap();
        assert_eq!(token.expose(), "tok-1");
        let error = Credential::Command(vec!["sh".into(), "-c".into(), "echo leak; exit 3".into()])
            .resolve()
            .unwrap_err();
        assert!(!error.contains("leak"), "{error}");
    }
}
