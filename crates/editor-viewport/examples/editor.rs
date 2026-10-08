use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use egui::{Context, Key, KeyboardShortcut, Modifiers};
use egui_wgpu::Renderer;
use pfx_editor_doc::{Changed, Files, History};
use pfx_editor_shell::{App, Pgpu, Script, Theme, Timing, Wake, Watch, gpu_turn, shot, window};
use pfx_editor_style::fixture;
use pfx_editor_style::widgets::{Header, KeyCap, Pill, Tabs, Tone};
use pfx_editor_toml::{NoCheck, Source};
use pfx_editor_viewport::{Event, Viewport, save};
use pfx_gpu::Gpu;

const KEYS: [(&str, &str); 8] = [
    ("G", "move"),
    ("R", "rotate"),
    ("S", "scale"),
    ("L", "space"),
    ("Ctrl", "snap"),
    ("F", "frame"),
    ("Home", "camera"),
    ("Ctrl+Z", "undo"),
];
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);

#[derive(Parser)]
#[command(about = "A small scene editor built from pfx's editor crates")]
struct Args {
    #[arg(help = "The scene.toml to edit [default: a copy of the sample scene]")]
    scene: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PNG",
        help = "Draw the editor offscreen into this PNG instead of opening a window"
    )]
    shot: Option<PathBuf>,
    #[arg(long, value_name = "WxH", default_value = "1280x720", value_parser = size, help = "The shot's size in pixels")]
    size: [u32; 2],
    #[arg(long, value_name = "NAME", help = "Select this object first")]
    select: Option<String>,
    #[arg(
        long,
        value_name = "FILE",
        help = "Drive the shot with this input script"
    )]
    input: Option<PathBuf>,
}

fn size(text: &str) -> Result<[u32; 2], String> {
    let parsed = text
        .split_once('x')
        .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]));
    parsed.ok_or_else(|| format!("expected WIDTHxHEIGHT, like 1280x720, not {text}"))
}

struct Editor {
    viewport: Viewport,
    sources: Vec<Source>,
    names: Vec<String>,
    tab: Option<usize>,
    files: Files,
    history: History,
    title: String,
}

impl Editor {
    fn open(scene: &Path) -> Result<Editor, String> {
        let viewport = Viewport::open("viewport", scene);
        let mut tomls = vec![scene.to_path_buf()];
        tomls.extend(
            viewport
                .files()
                .into_iter()
                .filter(|file| file != scene && file.extension().is_some_and(|ext| ext == "toml")),
        );
        let mut files = Files::new();
        let mut sources = Vec::new();
        let mut names = Vec::new();
        for file in tomls {
            files.load(&file).map_err(|error| error.to_string())?;
            let mut source = Source::new(("source", &file), &file);
            source.sync(&files, &NoCheck);
            names.push(name_of(&file));
            sources.push(source);
        }
        Ok(Editor {
            viewport,
            sources,
            title: name_of(scene),
            names,
            tab: Some(0),
            files,
            history: History::default(),
        })
    }

    fn select(&mut self, name: &str) -> bool {
        let found = self.viewport.select(Some(name)) && self.viewport.selection().is_some();
        if found {
            self.reveal(name);
        }
        found
    }

    fn reveal(&mut self, name: &str) {
        self.tab = Some(0);
        if let Some(range) = object_table(self.sources[0].text(), name) {
            self.sources[0].reveal(range);
        }
    }

    fn saved(&mut self, changed: Option<Changed>) {
        if let Some(changed) = changed {
            let _ = save(&mut self.files, &changed);
        }
    }
}

fn name_of(file: &Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn object_table(text: &str, name: &str) -> Option<Range<usize>> {
    let at = text.find(&format!("name = \"{name}\""))?;
    let start = text[..at].rfind("\n[").map_or(0, |line| line + 1);
    let end = text[at..].find("\n[").map_or(text.len(), |line| at + line);
    Some(start..text[..end].trim_end().len())
}

impl App for Editor {
    fn ui(&mut self, ctx: &Context) {
        let theme = Theme::of(ctx);
        let dirty = self
            .files
            .paths()
            .any(|path| self.files.get(path).is_some_and(|file| file.dirty()));
        let objects = self.viewport.scene().map_or(0, |scene| scene.objects.len());
        let info = format!("{objects} objects");
        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.add_space(theme.item_spacing);
            ui.horizontal(|ui| {
                let (word, tone) = if dirty {
                    ("edited", Tone::Busy)
                } else {
                    ("saved", Tone::Ok)
                };
                ui.add(Pill::new(word, tone));
                ui.add(Header::new("SC", &self.title).info(&info));
            });
            ui.add_space(theme.item_spacing);
        });
        egui::TopBottomPanel::bottom("keys").show(ctx, |ui| {
            ui.add_space(theme.item_spacing);
            ui.horizontal(|ui| {
                for (key, what) in KEYS {
                    ui.add(KeyCap::new(key));
                    ui.label(what);
                    ui.add_space(theme.item_spacing);
                }
            });
            ui.add_space(theme.item_spacing);
        });
        let width = ctx.content_rect().width() * 0.42;
        egui::SidePanel::right("sources")
            .default_width(width)
            .show(ctx, |ui| {
                ui.add_space(theme.item_spacing);
                let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
                ui.add(Tabs::new(&mut self.tab, &names));
                ui.add_space(theme.item_spacing);
                if let Some(source) = self.tab.and_then(|tab| self.sources.get_mut(tab)) {
                    let changed = source.show(
                        ui,
                        &mut self.files,
                        &mut self.history,
                        &NoCheck,
                        &mut |_, _| {},
                    );
                    self.saved(changed);
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                self.viewport.show(ui, &mut self.files, &mut self.history);
            });
        for event in self.viewport.take_events() {
            if let Event::Picked(Some(selection)) = event {
                self.reveal(&selection.name);
            }
        }
        if ctx.input_mut(|input| {
            input.consume_shortcut(&REDO) || input.consume_key(Modifiers::COMMAND, Key::Y)
        }) {
            let changed = self.history.redo(&mut self.files).ok().flatten();
            self.saved(changed);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&UNDO)) {
            let changed = self.history.undo(&mut self.files).ok().flatten();
            self.saved(changed);
        }
    }

    fn theme(&self) -> Theme {
        fixture::theme()
    }

    fn viewport(&self) -> bool {
        true
    }

    fn frame(&mut self, gpu: &Gpu, renderer: &mut Renderer, pixels_per_point: f32) -> bool {
        self.viewport.frame(gpu, renderer, pixels_per_point)
    }

    fn forget(&mut self) {
        self.viewport.forget();
    }

    fn label(&self) -> Option<String> {
        Some(self.title.clone())
    }

    fn watched(&self) -> Watch {
        Watch {
            files: self
                .sources
                .iter()
                .map(|source| source.file().to_path_buf())
                .collect(),
            ..Watch::default()
        }
    }

    fn changed(&mut self, _files: &[PathBuf]) {
        for source in &mut self.sources {
            source.reload(&mut self.files, &mut self.history, &NoCheck);
        }
    }

    fn waker(&mut self, wake: Wake) {
        self.viewport.set_wake(wake);
    }

    fn pgpu(&mut self, state: Pgpu) {
        self.viewport.set_pgpu(state);
    }

    fn timing(&self) -> Timing {
        self.viewport.timing()
    }
}

fn sample() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let from = root.join("tests/data/stand");
    let to = root.join("../../target/editor-example");
    std::fs::create_dir_all(&to).map_err(|error| format!("{}: {error}", to.display()))?;
    for entry in std::fs::read_dir(&from).map_err(|error| format!("{}: {error}", from.display()))? {
        let entry = entry.map_err(|error| error.to_string())?.path();
        let into = to.join(entry.file_name().unwrap_or_default());
        std::fs::copy(&entry, &into).map_err(|error| format!("{}: {error}", into.display()))?;
    }
    Ok(to.join("stand.scene.toml"))
}

fn run(args: Args) -> Result<(), String> {
    let scene = match args.scene {
        Some(scene) => scene,
        None => sample()?,
    };
    let mut editor = Editor::open(&scene)?;
    if let Some(name) = &args.select
        && !editor.select(name)
    {
        return Err(format!("no object named {name} in {}", scene.display()));
    }
    let Some(png) = args.shot else {
        return window(editor, None).map_err(|error| error.to_string());
    };
    let script = match &args.input {
        Some(file) => {
            let text = std::fs::read_to_string(file)
                .map_err(|error| format!("{}: {error}", file.display()))?;
            Script::parse(&text).map_err(|error| format!("{}: {error}", file.display()))?
        }
        None => Script::default(),
    };
    let since = Instant::now();
    let bytes = shot(&mut editor, args.size, &script).map_err(|error| error.to_string())?;
    gpu_turn(since);
    if let Some(failure) = editor.viewport.failure() {
        return Err(failure.to_string());
    }
    if let Some(refusal) = editor.viewport.refusal() {
        eprintln!("editor: {refusal}");
    }
    std::fs::write(&png, bytes).map_err(|error| format!("{}: {error}", png.display()))
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("editor: {error}");
            ExitCode::FAILURE
        }
    }
}
