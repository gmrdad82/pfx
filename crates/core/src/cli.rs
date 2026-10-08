use std::fmt;
use std::io::IsTerminal;

const RESET: &str = "\x1b[0m";
const ERROR: &str = "\x1b[1m\x1b[31m";
const INVALID: &str = "\x1b[33m";
const VALID: &str = "\x1b[32m";
const LITERAL: &str = "\x1b[1m";
const HEADER: &str = "\x1b[1m\x1b[4m";
const REQUIRED: &str = "the following required arguments were not provided:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub message: String,
    pub usage: Option<String>,
}

impl Refusal {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            usage: None,
        }
    }

    pub fn under(mut self, usage: impl Into<String>) -> Self {
        if self.usage.is_none() {
            self.usage = Some(usage.into());
        }
        self
    }

    pub fn render(&self, color: bool) -> String {
        let message = if color {
            style(&self.message)
        } else {
            self.message.clone()
        };
        let mut text = format!("{} {message}\n\n", paint("error:", ERROR, color));
        if let Some(usage) = self.usage.as_deref().filter(|_| !bare(&self.message)) {
            let usage = if color {
                style_usage(usage)
            } else {
                usage.to_string()
            };
            text.push_str(&format!("{} {usage}\n\n", paint("Usage:", HEADER, color)));
        }
        text.push_str(&format!(
            "For more information, try '{}'.",
            paint("--help", LITERAL, color)
        ));
        text
    }

    pub fn print(&self) {
        eprintln!("{}", self.render(color()));
    }
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for Refusal {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(false))
    }
}

pub fn color() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if set("NO_COLOR") {
        return false;
    }
    if set("CLICOLOR_FORCE") && std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
        return true;
    }
    !cfg!(windows)
        && std::io::stderr().is_terminal()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
        && std::env::var_os("CLICOLOR").is_none_or(|value| value != "0")
}

pub fn with_value(flag: &str) -> String {
    let name = flag
        .trim_start_matches('-')
        .replace('-', "_")
        .to_uppercase();
    format!("{flag} <{name}>")
}

pub fn unexpected(argument: &str) -> String {
    format!("unexpected argument '{argument}' found")
}

pub fn unrecognized(command: &str) -> String {
    format!("unrecognized subcommand '{command}'")
}

pub fn no_command(path: &str, commands: &[&str]) -> String {
    format!(
        "'{path}' requires a subcommand but one was not provided\n  [subcommands: {}]",
        commands.join(", ")
    )
}

pub fn repeated(argument: &str) -> String {
    format!("the argument '{argument}' cannot be used multiple times")
}

pub fn conflict(first: &str, second: &str) -> String {
    format!("the argument '{first}' cannot be used with '{second}'")
}

pub fn missing_value(flag: &str) -> String {
    format!(
        "a value is required for '{}' but none was supplied",
        with_value(flag)
    )
}

pub fn unexpected_value(value: &str, flag: &str) -> String {
    format!("unexpected value '{value}' for '{flag}' found; no more were expected")
}

pub fn required<S: AsRef<str>>(arguments: &[S]) -> String {
    let mut text = REQUIRED.to_string();
    for argument in arguments {
        text.push_str("\n  ");
        text.push_str(argument.as_ref());
    }
    text
}

pub fn invalid(value: &str, flag: &str, reason: impl fmt::Display) -> String {
    format!(
        "invalid value '{value}' for '{}': {reason}",
        with_value(flag)
    )
}

pub fn invalid_choice(value: &str, flag: &str, choices: &[&str]) -> String {
    format!(
        "invalid value '{value}' for '{}'\n  [possible values: {}]",
        with_value(flag),
        choices.join(", ")
    )
}

fn bare(message: &str) -> bool {
    message.starts_with("a value is required for ") || message.starts_with("invalid value ")
}

fn paint(text: &str, style: &str, color: bool) -> String {
    if color {
        format!("{style}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn quotes(line: &str, tone: impl Fn(usize, &str) -> &'static str) -> String {
    let parts: Vec<&str> = line.split('\'').collect();
    if parts.len().is_multiple_of(2) {
        return line.to_string();
    }
    let mut text = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index.is_multiple_of(2) {
            text.push_str(part);
        } else {
            text.push('\'');
            text.push_str(&paint(part, tone(index / 2, parts[index - 1]), true));
            text.push('\'');
        }
    }
    text
}

fn style(message: &str) -> String {
    let mut listing = false;
    let mut lines = Vec::new();
    for (index, line) in message.split('\n').enumerate() {
        if index == 0 {
            listing = line == REQUIRED;
            lines.push(quotes(line, |nth, before| {
                if nth > 0 && before.ends_with("for ") {
                    LITERAL
                } else {
                    INVALID
                }
            }));
        } else if let Some(tip) = line.strip_prefix("  tip:") {
            lines.push(format!(
                "  {}{}",
                paint("tip:", VALID, true),
                quotes(tip, |_, _| VALID)
            ));
        } else if let Some((label, items)) = line
            .strip_prefix("  [")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|inner| inner.split_once(": "))
        {
            let items: Vec<String> = items
                .split(", ")
                .map(|item| paint(item, VALID, true))
                .collect();
            lines.push(format!("  [{label}: {}]", items.join(", ")));
        } else if let Some(item) = line.strip_prefix("  ").filter(|_| listing) {
            lines.push(format!("  {}", paint(item, VALID, true)));
        } else {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

fn style_usage(usage: &str) -> String {
    let words: Vec<&str> = usage.split(' ').collect();
    let path = words
        .iter()
        .take_while(|word| word.starts_with(|c: char| c.is_alphanumeric()))
        .count()
        .max(1);
    let mut parts = vec![paint(&words[..path].join(" "), LITERAL, true)];
    for word in &words[path..] {
        if word.starts_with('-') {
            parts.push(paint(word, LITERAL, true));
        } else {
            parts.push(word.to_string());
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_reads_and_colours_like_clap() {
        let tui = Refusal::new(unrecognized("tui")).under("pfx <COMMAND>");
        assert_eq!(
            tui.render(false),
            "error: unrecognized subcommand 'tui'\n\nUsage: pfx <COMMAND>\n\nFor more information, try '--help'."
        );
        assert_eq!(
            tui.render(true),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m unrecognized subcommand '\u{1b}[33mtui\u{1b}[0m'\n\n\u{1b}[1m\u{1b}[4mUsage:\u{1b}[0m \u{1b}[1mpfx\u{1b}[0m <COMMAND>\n\nFor more information, try '\u{1b}[1m--help\u{1b}[0m'."
        );
        let missing = Refusal::new(missing_value("--out")).under("pfx bake [OPTIONS]");
        assert_eq!(
            missing.render(false),
            "error: a value is required for '--out <OUT>' but none was supplied\n\nFor more information, try '--help'."
        );
        assert_eq!(
            Refusal::new(no_command("pfx steam", &["stage"]))
                .under("pfx steam <COMMAND>")
                .render(true),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m '\u{1b}[33mpfx steam\u{1b}[0m' requires a subcommand but one was not provided\n  [subcommands: \u{1b}[32mstage\u{1b}[0m]\n\n\u{1b}[1m\u{1b}[4mUsage:\u{1b}[0m \u{1b}[1mpfx steam\u{1b}[0m <COMMAND>\n\nFor more information, try '\u{1b}[1m--help\u{1b}[0m'."
        );
        assert_eq!(
            Refusal::new(required(&[with_value("--scene")]))
                .under("pfx render [OPTIONS] --scene <SCENE> <--clip|--still>")
                .render(true),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m the following required arguments were not provided:\n  \u{1b}[32m--scene <SCENE>\u{1b}[0m\n\n\u{1b}[1m\u{1b}[4mUsage:\u{1b}[0m \u{1b}[1mpfx render\u{1b}[0m [OPTIONS] \u{1b}[1m--scene\u{1b}[0m <SCENE> <--clip|--still>\n\nFor more information, try '\u{1b}[1m--help\u{1b}[0m'."
        );
        assert_eq!(
            Refusal::new(invalid("x", "--samples", "invalid digit found in string")).render(true),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m invalid value '\u{1b}[33mx\u{1b}[0m' for '\u{1b}[1m--samples <SAMPLES>\u{1b}[0m': invalid digit found in string\n\nFor more information, try '\u{1b}[1m--help\u{1b}[0m'."
        );
        assert_eq!(
            Refusal::new(format!(
                "{}\n\n  tip: a similar argument exists: '--scene'",
                unexpected("--scen")
            ))
            .under("pfx render [OPTIONS]")
            .render(true),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m unexpected argument '\u{1b}[33m--scen\u{1b}[0m' found\n\n  \u{1b}[32mtip:\u{1b}[0m a similar argument exists: '\u{1b}[32m--scene\u{1b}[0m'\n\n\u{1b}[1m\u{1b}[4mUsage:\u{1b}[0m \u{1b}[1mpfx render\u{1b}[0m [OPTIONS]\n\nFor more information, try '\u{1b}[1m--help\u{1b}[0m'."
        );
        assert_eq!(
            Refusal::new(conflict("--clip", "--still")).render(true),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m the argument '\u{1b}[33m--clip\u{1b}[0m' cannot be used with '\u{1b}[33m--still\u{1b}[0m'\n\nFor more information, try '\u{1b}[1m--help\u{1b}[0m'."
        );
    }
}
