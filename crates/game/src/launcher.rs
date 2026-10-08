use std::process::ExitCode;
use std::rc::Rc;

#[cfg(feature = "tools")]
use pfx_core::cli::{self, Refusal};
use pfx_play::Game;

use crate::config::Config;

pub type Command = Box<dyn Fn(&[String]) -> Result<ExitCode, CommandError>>;
pub type Factory = Rc<dyn Fn() -> Box<dyn Game>>;
pub type Edit = Box<dyn FnOnce(Factory, &[String]) -> Result<(), String>>;
pub type Rest = Box<dyn FnOnce(&[String]) -> Result<Unmatched, CommandError>>;

pub const USAGE: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError {
    pub code: u8,
    pub message: String,
}

impl CommandError {
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(USAGE, message)
    }
}

impl<E: std::fmt::Display> From<E> for CommandError {
    fn from(error: E) -> Self {
        Self::new(1, error.to_string())
    }
}

pub trait Outcome {
    fn outcome(self) -> Result<ExitCode, CommandError>;
}

impl Outcome for () {
    fn outcome(self) -> Result<ExitCode, CommandError> {
        Ok(ExitCode::SUCCESS)
    }
}

impl Outcome for ExitCode {
    fn outcome(self) -> Result<ExitCode, CommandError> {
        Ok(self)
    }
}

impl Outcome for u8 {
    fn outcome(self) -> Result<ExitCode, CommandError> {
        Ok(ExitCode::from(self))
    }
}

impl<T: Outcome, E: Into<CommandError>> Outcome for Result<T, E> {
    fn outcome(self) -> Result<ExitCode, CommandError> {
        self.map_err(Into::into).and_then(Outcome::outcome)
    }
}

#[derive(Debug, PartialEq)]
pub enum Unmatched {
    Run,
    Command(Vec<String>),
    Exit(ExitCode),
}

pub const BUILT_IN: [&str; 3] = ["edit", "help", "version"];

pub const TOOLS_MARKER: &str =
    "pfx-game tools build: the launcher's commands and pfx's editor are linked in";

#[derive(Default)]
pub struct Commands {
    #[cfg(feature = "tools")]
    list: Vec<(String, String, Command)>,
    #[cfg(feature = "tools")]
    edit: Option<(String, Edit)>,
    #[cfg(feature = "tools")]
    problems: Vec<String>,
    #[cfg(feature = "tools")]
    rest: Option<Rest>,
    #[cfg(feature = "tools")]
    shapes: Vec<(String, String)>,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "tools")]
    pub fn add<R: Outcome>(
        &mut self,
        name: &str,
        about: &str,
        command: impl Fn(&[String]) -> R + 'static,
    ) -> &mut Self {
        if let Some(problem) = self.refuse(name) {
            self.problems.push(problem);
        } else {
            self.list.push((
                name.to_string(),
                about.to_string(),
                Box::new(move |args: &[String]| command(args).outcome()),
            ));
        }
        self
    }

    #[cfg(not(feature = "tools"))]
    pub fn add<R: Outcome>(
        &mut self,
        _name: &str,
        _about: &str,
        _command: impl Fn(&[String]) -> R + 'static,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "tools")]
    pub fn usage(&mut self, command: &str, shape: &str) -> &mut Self {
        self.shapes.retain(|(name, _)| name != command);
        self.shapes.push((command.to_string(), shape.to_string()));
        self
    }

    #[cfg(not(feature = "tools"))]
    pub fn usage(&mut self, _command: &str, _shape: &str) -> &mut Self {
        self
    }

    #[cfg(feature = "tools")]
    fn shape(&self, command: &str) -> Option<&str> {
        self.shapes
            .iter()
            .find(|(name, _)| name == command)
            .map(|(_, shape)| shape.as_str())
    }

    #[cfg(feature = "tools")]
    fn usage_line(&self, name: &str, command: &str) -> String {
        let shape = self.shape(command).unwrap_or("[ARGS]...");
        format!("{name} {command} {shape}").trim_end().to_string()
    }

    #[cfg(feature = "tools")]
    fn takes_nothing(&self, command: &str, args: &[String]) -> Option<String> {
        match (self.shape(command), args.first()) {
            (Some(""), Some(extra)) => Some(cli::unexpected(extra)),
            _ => None,
        }
    }

    #[cfg(feature = "tools")]
    pub fn unmatched(
        &mut self,
        hook: impl FnOnce(&[String]) -> Result<Unmatched, CommandError> + 'static,
    ) -> &mut Self {
        self.rest = Some(Box::new(hook));
        self
    }

    #[cfg(not(feature = "tools"))]
    pub fn unmatched(
        &mut self,
        _hook: impl FnOnce(&[String]) -> Result<Unmatched, CommandError> + 'static,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "tools")]
    pub fn edit(
        &mut self,
        about: &str,
        hook: impl FnOnce(Factory, &[String]) -> Result<(), String> + 'static,
    ) -> &mut Self {
        self.edit = Some((about.to_string(), Box::new(hook)));
        self
    }

    #[cfg(not(feature = "tools"))]
    pub fn edit(
        &mut self,
        _about: &str,
        _hook: impl FnOnce(Factory, &[String]) -> Result<(), String> + 'static,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "tools")]
    fn refuse(&self, name: &str) -> Option<String> {
        if name.is_empty() || name.starts_with('-') || name.chars().any(char::is_whitespace) {
            return Some(format!(
                "command {name:?}: a name is one word that does not start with '-'"
            ));
        }
        if BUILT_IN.contains(&name) {
            return Some(format!("command {name:?}: that name is built in"));
        }
        if self.list.iter().any(|(taken, _, _)| taken == name) {
            return Some(format!("command {name:?}: registered twice"));
        }
        None
    }

    #[cfg(feature = "tools")]
    pub fn names(&self) -> Vec<&str> {
        self.list.iter().map(|(name, _, _)| name.as_str()).collect()
    }

    #[cfg(feature = "tools")]
    pub fn help(&self, name: &str, version: &str) -> String {
        let mut rows: Vec<(&str, &str)> = Vec::new();
        if let Some((about, _)) = &self.edit {
            rows.push(("edit", about));
        }
        rows.push(("help", "lists these commands"));
        rows.push(("version", "prints the game's version"));
        rows.extend(
            self.list
                .iter()
                .map(|(name, about, _)| (name.as_str(), about.as_str())),
        );
        let width = rows.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
        let mut text = format!(
            "{name} {version}\n\nUsage: {name} [COMMAND]\n\nWith no command, {name} runs the game.\n\nCommands:\n"
        );
        for (command, about) in rows {
            text.push_str(&format!("  {command:<width$}  {about}\n"));
        }
        text
    }
}

pub enum Decision {
    Run,
    Done(ExitCode),
    #[cfg(feature = "tools")]
    Edit(Edit, Vec<String>),
}

impl std::fmt::Debug for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decision::Run => f.write_str("Run"),
            Decision::Done(code) => write!(f, "Done({code:?})"),
            #[cfg(feature = "tools")]
            Decision::Edit(_, args) => write!(f, "Edit({args:?})"),
        }
    }
}

#[cfg(feature = "tools")]
pub fn decide(
    args: &[String],
    name: &str,
    version: &str,
    register: impl FnOnce(&mut Commands),
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> Decision {
    decide_in(args, name, version, register, out, err, false)
}

#[cfg(feature = "tools")]
fn decide_in(
    args: &[String],
    name: &str,
    version: &str,
    register: impl FnOnce(&mut Commands),
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
    color: bool,
) -> Decision {
    std::hint::black_box(TOOLS_MARKER);
    if args.is_empty() {
        return Decision::Run;
    }
    let mut commands = Commands::new();
    register(&mut commands);
    for (command, _) in &commands.shapes {
        if command != "edit" && !commands.list.iter().any(|(taken, _, _)| taken == command) {
            commands
                .problems
                .push(format!("usage {command:?}: no such command"));
        }
    }
    if !commands.problems.is_empty() {
        for problem in &commands.problems {
            let _ = writeln!(err, "{name}: {problem}");
        }
        return Decision::Done(ExitCode::from(USAGE));
    }
    let mut talk = Talk {
        name,
        version,
        out,
        err,
        color,
    };
    route(&mut commands, args, &mut talk)
}

#[cfg(feature = "tools")]
struct Talk<'a, O, E> {
    name: &'a str,
    version: &'a str,
    out: &'a mut O,
    err: &'a mut E,
    color: bool,
}

#[cfg(feature = "tools")]
impl<O: std::io::Write, E: std::io::Write> Talk<'_, O, E> {
    fn refuse(&mut self, message: String, usage: String) -> Decision {
        let refusal = Refusal::new(message).under(usage);
        let _ = writeln!(self.err, "{}", refusal.render(self.color));
        Decision::Done(ExitCode::from(USAGE))
    }

    fn top(&self) -> String {
        format!("{} [COMMAND]", self.name)
    }
}

#[cfg(feature = "tools")]
fn route<O: std::io::Write, E: std::io::Write>(
    commands: &mut Commands,
    args: &[String],
    talk: &mut Talk<'_, O, E>,
) -> Decision {
    let Some((first, rest)) = args.split_first() else {
        return Decision::Run;
    };
    let name = talk.name;
    match first.as_str() {
        "help" | "--help" | "-h" => {
            let _ = write!(talk.out, "{}", commands.help(name, talk.version));
            Decision::Done(ExitCode::SUCCESS)
        }
        "version" | "--version" | "-V" => {
            let _ = writeln!(talk.out, "{name} {}", talk.version);
            Decision::Done(ExitCode::SUCCESS)
        }
        "edit" => match (commands.takes_nothing("edit", rest), commands.edit.take()) {
            (_, None) => talk.refuse(cli::unrecognized("edit"), talk.top()),
            (Some(message), Some(_)) => talk.refuse(message, commands.usage_line(name, "edit")),
            (None, Some((_, hook))) => Decision::Edit(hook, rest.to_vec()),
        },
        command => {
            if let Some((_, _, run)) = commands.list.iter().find(|(taken, _, _)| taken == command) {
                if let Some(message) = commands.takes_nothing(command, rest) {
                    return talk.refuse(message, commands.usage_line(name, command));
                }
                return match run(rest) {
                    Ok(code) => Decision::Done(code),
                    Err(error) if error.code == USAGE => {
                        talk.refuse(error.message, commands.usage_line(name, command))
                    }
                    Err(error) => {
                        let _ = writeln!(talk.err, "{name} {command}: {}", error.message);
                        Decision::Done(ExitCode::from(error.code))
                    }
                };
            }
            match commands.rest.take() {
                Some(hook) => match hook(args) {
                    Ok(Unmatched::Run) => Decision::Run,
                    Ok(Unmatched::Exit(code)) => Decision::Done(code),
                    Ok(Unmatched::Command(again)) => route(commands, &again, talk),
                    Err(error) => {
                        let _ = writeln!(talk.err, "{name}: {}", error.message);
                        Decision::Done(ExitCode::from(error.code))
                    }
                },
                None if command.starts_with('-') => {
                    talk.refuse(cli::unexpected(command), talk.top())
                }
                None => talk.refuse(cli::unrecognized(command), talk.top()),
            }
        }
    }
}

#[cfg(not(feature = "tools"))]
pub fn decide(
    _args: &[String],
    _name: &str,
    _version: &str,
    _register: impl FnOnce(&mut Commands),
    _out: &mut impl std::io::Write,
    _err: &mut impl std::io::Write,
) -> Decision {
    Decision::Run
}

pub fn factory<G: Game + 'static>(make: impl Fn() -> G + 'static) -> Factory {
    Rc::new(move || Box::new(make()) as Box<dyn Game>)
}

fn play<G: Game + 'static>(game: G, config: Config) -> ExitCode {
    match crate::host::run(game, config) {
        Ok(code) => ExitCode::from(code),
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(feature = "tools")]
pub fn edit(name: &str, hook: Edit, factory: Factory, args: &[String]) -> ExitCode {
    match hook(factory, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{name} edit: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "tools")]
pub fn launch<G: Game + 'static>(
    make: impl Fn() -> G + 'static,
    config: Config,
    register: impl FnOnce(&mut Commands),
) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let decision = decide_in(
        &args,
        &config.name,
        &config.version,
        register,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
        cli::color(),
    );
    match decision {
        Decision::Run => play(make(), config),
        Decision::Done(code) => code,
        Decision::Edit(hook, rest) => edit(&config.name, hook, factory(make), &rest),
    }
}

#[cfg(not(feature = "tools"))]
pub fn launch<G: Game + 'static>(
    make: impl Fn() -> G + 'static,
    config: Config,
    _register: impl FnOnce(&mut Commands),
) -> ExitCode {
    play(make(), config)
}
