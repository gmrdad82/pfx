use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use egui::{
    Align, Align2, Color32, Context, CornerRadius, Frame, Id, Key, Layout, Margin, RichText,
    Stroke, Ui, Vec2,
};

use crate::shell::{code, list_row, small, tail_text, text_field};
use crate::theme::*;
use crate::widgets::KeyCap;

pub const LIMIT: usize = 10;
const CARD: f32 = 520.0;
const LIST: f32 = 280.0;
const FILE: &str = "\u{f15b}";
const FOLDER: &str = "\u{f07b}";

pub fn field_id() -> Id {
    Id::new("workbench open path")
}

pub fn name_id() -> Id {
    Id::new("workbench save name")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Purpose {
    #[default]
    Open,
    Import,
    SaveAs,
    Export,
}

impl Purpose {
    fn saves(self) -> bool {
        matches!(self, Purpose::SaveAs | Purpose::Export)
    }

    fn title(self) -> &'static str {
        match self {
            Purpose::Open => "OPEN",
            Purpose::Import => "IMPORT GLTF",
            Purpose::SaveAs => "SAVE AS .PFX",
            Purpose::Export => "EXPORT GLTF",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Purpose::Open => "open",
            Purpose::Import => "import",
            Purpose::SaveAs => "save",
            Purpose::Export => "export",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Purpose::Open => "path of a recipe, a scene, a .pitofx or a .glb",
            Purpose::Import => "path of a .glb",
            Purpose::SaveAs | Purpose::Export => "folder",
        }
    }

    fn accepts(self, path: &Path) -> bool {
        let kind = crate::file::kind(path);
        path.is_file()
            && visible_name(path).is_some()
            && match self {
                Purpose::Open => kind.is_some() || pfx_file::is_legacy_path(path),
                Purpose::Import | Purpose::Export => kind == Some(crate::file::Kind::Glb),
                Purpose::SaveAs => kind == Some(crate::file::Kind::Pfx),
            }
    }
}

#[derive(Clone, Default)]
struct Thumbs(std::collections::BTreeMap<PathBuf, Option<egui::TextureHandle>>);

impl std::fmt::Debug for Thumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl Thumbs {
    fn get(&mut self, ctx: &Context, path: &Path) -> Option<egui::TextureId> {
        if crate::file::kind(path) != Some(crate::file::Kind::Pfx) {
            return None;
        }
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let image = crate::file::decode(&crate::file::thumb(path)?)?;
                Some(ctx.load_texture(
                    format!("thumb {}", path.display()),
                    image,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .as_ref()
            .map(egui::TextureHandle::id)
    }
}

pub fn state_dir() -> PathBuf {
    own_dir("XDG_STATE_HOME", state_dir_from)
}

fn own_dir(xdg: &str, linux: fn(Option<&OsStr>, Option<&OsStr>) -> PathBuf) -> PathBuf {
    let home = home();
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|dir| !dir.is_empty()) {
            return PathBuf::from(local).join("pfx");
        }
    } else if cfg!(target_os = "macos")
        && let Some(home) = &home
    {
        return PathBuf::from(home).join("Library/Application Support/pfx");
    }
    linux(std::env::var_os(xdg).as_deref(), home.as_deref())
}

fn home() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()))
}

pub fn state_dir_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    match xdg {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("pfx"),
        _ => home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state/pfx"),
    }
}

pub fn data_dir_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    match xdg {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("pfx"),
        _ => home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/pfx"),
    }
}

pub fn installed_samples() -> PathBuf {
    own_dir("XDG_DATA_HOME", data_dir_from).join("samples")
}

pub fn samples(fallback: Option<&Path>) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    samples_with(exe.as_deref().and_then(Path::parent), fallback)
}

pub fn samples_with(start: Option<&Path>, fallback: Option<&Path>) -> Option<PathBuf> {
    start
        .and_then(samples_from)
        .or_else(|| fallback.filter(|dir| dir.is_dir()).map(Path::to_path_buf))
}

pub fn samples_from(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    for _ in 0..8 {
        let candidate = dir.join("samples");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

pub fn known(
    anchor: Option<&Path>,
    last: Option<&Path>,
    samples: Option<&Path>,
) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut push = |title: &str, path: &Path| {
        if path.as_os_str().is_empty() || !path.is_dir() {
            return;
        }
        if out.iter().any(|(_, known)| known == path) {
            return;
        }
        let title = if samples.is_some_and(|dir| dir == path) {
            "samples".to_string()
        } else {
            title.to_string()
        };
        out.push((title, path.to_path_buf()));
    };
    if let Some(path) = anchor {
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "folder".to_string());
        push(&title, path);
    }
    if let Some(path) = samples {
        push("samples", path);
    }
    if let Some(path) = last {
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "folder".to_string());
        push(&title, path);
    }
    out
}

#[cfg(test)]
pub fn recipes(folder: &Path) -> Vec<PathBuf> {
    files(folder, Purpose::Open)
}

fn files(folder: &Path, purpose: Purpose) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if purpose.accepts(&path) {
            found.push(path);
        }
    }
    found.sort_by(|a, b| by_name(a, b));
    found
}

#[cfg(test)]
pub fn complete(typed: &str) -> Vec<PathBuf> {
    complete_for(typed, Purpose::Open)
}

fn complete_for(typed: &str, purpose: Purpose) -> Vec<PathBuf> {
    if typed.is_empty() {
        return Vec::new();
    }
    let (dir, prefix) = split_query(typed);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = visible_name(&path) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        if path.is_dir() || purpose.accepts(&path) {
            found.push(path);
        }
    }
    found.sort_by(|a, b| by_name(a, b));
    found
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Recipe(PathBuf),
    Dir(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub recent: Vec<PathBuf>,
    pub groups: Vec<(String, Vec<PathBuf>)>,
    pub matches: Vec<Row>,
}

impl Listed {
    fn len(&self) -> usize {
        if !self.matches.is_empty() {
            return self.matches.len();
        }
        self.recent.len()
            + self
                .groups
                .iter()
                .map(|(_, recipes)| recipes.len())
                .sum::<usize>()
    }

    fn get(&self, index: usize) -> Option<Row> {
        if !self.matches.is_empty() {
            return self.matches.get(index).cloned();
        }
        if index < self.recent.len() {
            return Some(Row::Recipe(self.recent[index].clone()));
        }
        let mut index = index - self.recent.len();
        for (_, recipes) in &self.groups {
            if index < recipes.len() {
                return Some(Row::Recipe(recipes[index].clone()));
            }
            index -= recipes.len();
        }
        None
    }
}

#[cfg(test)]
pub fn listed(query: &str, recent: &[PathBuf], folders: &[(String, PathBuf)]) -> Listed {
    listed_for(query, recent, folders, Purpose::Open)
}

fn listed_for(
    query: &str,
    recent: &[PathBuf],
    folders: &[(String, PathBuf)],
    purpose: Purpose,
) -> Listed {
    let query = query.trim();
    if !query.is_empty() {
        return Listed {
            recent: Vec::new(),
            groups: Vec::new(),
            matches: complete_for(query, purpose)
                .into_iter()
                .map(|path| {
                    if path.is_dir() {
                        Row::Dir(path)
                    } else {
                        Row::Recipe(path)
                    }
                })
                .collect(),
        };
    }
    Listed {
        recent: recent
            .iter()
            .filter(|_| purpose == Purpose::Open)
            .take(LIMIT)
            .cloned()
            .collect(),
        groups: folders
            .iter()
            .map(|(title, folder)| (title.clone(), files(folder, purpose)))
            .collect(),
        matches: Vec::new(),
    }
}

fn sources(
    live: bool,
    path: &str,
    anchor: Option<&Path>,
    book: &Book,
    fallback: Option<&Path>,
    own: bool,
) -> (Vec<(String, PathBuf)>, Vec<PathBuf>) {
    let recent = book.list();
    let last = if live {
        Path::new(path).parent().map(Path::to_path_buf)
    } else {
        recent
            .first()
            .and_then(|item| item.parent().map(Path::to_path_buf))
    };
    (
        known(
            anchor,
            last.as_deref(),
            samples(fallback).filter(|_| !own).as_deref(),
        ),
        recent,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Open(PathBuf),
    Browse(PathBuf),
}

pub fn choose(query: &str, selected: Option<&Row>) -> Option<Choice> {
    choose_with(query, selected, home().as_deref())
}

fn choose_with(query: &str, selected: Option<&Row>, home: Option<&OsStr>) -> Option<Choice> {
    let query = query.trim();
    let typed = expand_home(query, home);
    if !query.is_empty() && typed.is_file() {
        return Some(Choice::Open(typed));
    }
    if !query.is_empty() && typed.is_dir() {
        return Some(Choice::Browse(typed));
    }
    match selected {
        Some(Row::Recipe(path)) => Some(Choice::Open(path.clone())),
        Some(Row::Dir(path)) => Some(Choice::Browse(path.clone())),
        None => None,
    }
}

pub const SETTINGS: &str = "settings.pitofx";
pub const SETTINGS_TOML: &str = "settings.toml";

#[derive(Clone, Debug)]
pub struct Book {
    path: Option<PathBuf>,
}

impl Book {
    pub fn machine() -> Book {
        Book {
            path: Some(state_dir().join(SETTINGS)),
        }
    }

    pub fn empty() -> Book {
        Book { path: None }
    }

    #[cfg(test)]
    pub fn at(dir: &Path) -> Book {
        Book {
            path: Some(dir.join(SETTINGS)),
        }
    }

    fn settings(&self) -> toml_edit::DocumentMut {
        self.path
            .as_ref()
            .and_then(|path| {
                pfx_file::Reader::open(path).ok().or_else(|| {
                    pfx_file::Reader::open(path.with_extension(pfx_file::LEGACY_EXTENSION)).ok()
                })
            })
            .and_then(|reader| {
                reader
                    .named(pfx_file::FILE, SETTINGS_TOML)
                    .ok()
                    .flatten()
                    .map(|bytes| bytes.into_owned())
            })
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| text.parse().ok())
            .unwrap_or_default()
    }

    fn keep(&self, settings: &toml_edit::DocumentMut) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        pfx_file::Writer::new()
            .named(
                pfx_file::FILE,
                SETTINGS_TOML,
                settings.to_string().into_bytes(),
                pfx_file::Codec::Auto,
            )
            .write(path)
            .map_err(|error| std::io::Error::other(error.to_string()))
    }

    pub fn list(&self) -> Vec<PathBuf> {
        self.settings()
            .get("recent")
            .and_then(toml_edit::Item::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml_edit::Value::as_str)
            .filter(|line| !line.trim().is_empty())
            .map(PathBuf::from)
            .collect()
    }

    pub fn remember(&self, recipe: &Path) -> std::io::Result<()> {
        if self.path.is_none() {
            return Ok(());
        }
        let recipe = std::path::absolute(recipe).unwrap_or_else(|_| recipe.to_path_buf());
        let mut paths = self.list();
        paths.retain(|path| !same_path(path, &recipe));
        paths.insert(0, recipe);
        paths.truncate(LIMIT);
        let mut settings = self.settings();
        let recent: toml_edit::Array = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        settings["recent"] = toml_edit::value(recent);
        self.keep(&settings)
    }

    pub fn view(&self) -> crate::wire::Look {
        let mut look = crate::wire::Look::default();
        let settings = self.settings();
        let Some(view) = settings
            .get("view")
            .and_then(toml_edit::Item::as_table_like)
        else {
            return look;
        };
        let on = |key: &str| {
            view.get(key)
                .and_then(toml_edit::Item::as_bool)
                .is_some_and(|on| on)
        };
        look.amount = if on("wireframe") { 1.0 } else { 0.0 };
        look.xray = on("xray");
        look.clay = if on("clay") { 1.0 } else { 0.0 };
        look
    }

    pub fn keep_view(&self, look: crate::wire::Look) -> std::io::Result<()> {
        if self.path.is_none() {
            return Ok(());
        }
        let mut settings = self.settings();
        let mut view = toml_edit::Table::new();
        view["wireframe"] = toml_edit::value(look.on());
        view["xray"] = toml_edit::value(look.xray);
        view["clay"] = toml_edit::value(look.clay > 0.0);
        settings["view"] = toml_edit::Item::Table(view);
        self.keep(&settings)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Open(PathBuf),
    Import(PathBuf),
    SaveAs(PathBuf),
    Export(PathBuf, bool),
    Close,
}

#[derive(Clone, Debug)]
struct Held {
    query: String,
    anchor: Option<PathBuf>,
    listed: Listed,
}

#[derive(Clone, Debug, Default)]
pub struct Picker {
    query: String,
    seen: String,
    shown: bool,
    anchor: Option<PathBuf>,
    cursor: usize,
    error: Option<String>,
    focus: bool,
    held: Option<Held>,
    fallback: Option<PathBuf>,
    own: bool,
    purpose: Purpose,
    name: String,
    selected: Option<bool>,
    thumbs: Thumbs,
}

impl Picker {
    pub fn set_samples_fallback(&mut self, folder: PathBuf) {
        self.fallback = Some(folder);
    }

    pub fn without_samples(&mut self) {
        self.own = true;
    }

    pub fn on_folder(folder: &str) -> Picker {
        Picker {
            anchor: Some(PathBuf::from(folder)),
            shown: true,
            ..Picker::default()
        }
    }

    pub fn showing(&self, live: bool) -> bool {
        !live || self.shown
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn show(&mut self) {
        if self.purpose != Purpose::Open {
            self.close();
        }
        self.shown = true;
        self.focus = true;
    }

    pub fn show_for(
        &mut self,
        purpose: Purpose,
        folder: Option<PathBuf>,
        name: String,
        selected: Option<bool>,
    ) {
        self.close();
        self.purpose = purpose;
        if let Some(folder) = folder.filter(|folder| folder.is_dir()) {
            self.browse_to(folder);
        }
        self.name = name;
        self.selected = selected;
        self.shown = true;
        self.focus = true;
    }

    pub fn close(&mut self) {
        self.shown = false;
        self.query.clear();
        self.seen.clear();
        self.error = None;
        self.cursor = 0;
        self.focus = false;
        self.held = None;
        self.purpose = Purpose::Open;
        self.name.clear();
        self.selected = None;
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn draw(&mut self, ctx: &Context, live: bool, path: &str, book: &Book) -> Option<Act> {
        let palette = Palette::of(ctx);
        if !self.showing(live) {
            self.held = None;
            return None;
        }
        egui::Area::new(Id::new("workbench open"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fade_in(false)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(palette.bg_base)
                    .stroke(Stroke::new(STROKE, palette.line_strong))
                    .corner_radius(CornerRadius::same(RADIUS))
                    .inner_margin(Margin::same(PANEL_PADDING))
                    .show(ui, |ui| {
                        ui.set_min_width(CARD);
                        ui.set_max_width(CARD);
                        ui.spacing_mut().item_spacing = Vec2::new(ITEM_SPACING, ITEM_SPACING);
                        self.contents(ui, live, path, book)
                    })
                    .inner
            })
            .inner
    }

    fn hold(&mut self, live: bool, path: &str, book: &Book) {
        let same = self
            .held
            .as_ref()
            .is_some_and(|held| held.query == self.query && held.anchor == self.anchor);
        if same {
            return;
        }
        let (folders, recent) = if self.query.trim().is_empty() {
            sources(
                live,
                path,
                self.anchor.as_deref(),
                book,
                self.fallback.as_deref(),
                self.own,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let listed = listed_for(&self.query, &recent, &folders, self.purpose);
        self.held = Some(Held {
            query: self.query.clone(),
            anchor: self.anchor.clone(),
            listed,
        });
    }

    fn contents(&mut self, ui: &mut Ui, live: bool, path: &str, book: &Book) -> Option<Act> {
        let palette = Palette::of(ui.ctx());
        let purpose = self.purpose;
        if self.focus {
            let id = if purpose.saves() {
                name_id()
            } else {
                field_id()
            };
            ui.memory_mut(|memory| memory.request_focus(id));
            if !ui.is_sizing_pass() {
                self.focus = false;
            }
        }
        title(ui, purpose);
        let mut response = text_field(
            ui,
            &mut self.query,
            purpose.hint(),
            ui.available_width(),
            Some(field_id()),
        );
        if purpose.saves() {
            let hint = if purpose == Purpose::SaveAs {
                "name.pitofx"
            } else {
                "name.glb"
            };
            let named = text_field(
                ui,
                &mut self.name,
                hint,
                ui.available_width(),
                Some(name_id()),
            );
            if named.has_focus() || named.lost_focus() {
                response = named;
            }
            if let Some(selected) = self.selected.as_mut() {
                ui.checkbox(
                    selected,
                    small(ui, "the selected object only", palette.text_secondary),
                );
            }
        }
        if self.query != self.seen {
            self.cursor = 0;
            self.seen.clone_from(&self.query);
            self.error = None;
        }
        self.hold(live, path, book);
        let len = self
            .held
            .as_ref()
            .map(|held| held.listed.len())
            .unwrap_or(0);
        if len > 0 {
            self.cursor = self.cursor.min(len - 1);
        }
        let take = response.has_focus() || response.lost_focus() || self.shown;
        let enter = take && pressed(ui, Key::Enter);
        let escape = take && pressed(ui, Key::Escape);
        let up = take && pressed(ui, Key::ArrowUp);
        let down = take && pressed(ui, Key::ArrowDown);
        if up {
            self.cursor = self.cursor.saturating_sub(1);
        }
        if down && len > 0 {
            self.cursor = (self.cursor + 1).min(len - 1);
        }
        if let Some(error) = &self.error {
            ui.label(code(ui, crate::shown::text(error), palette.danger));
        }
        let cursor = self.cursor;
        let empty = self.query.trim().is_empty();
        let mut clicked = None;
        let thumbs = &mut self.thumbs;
        if let Some(held) = self.held.as_ref() {
            let listed = &held.listed;
            egui::ScrollArea::vertical()
                .id_salt("workbench open list")
                .max_height(LIST)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if empty {
                        let mut index = 0;
                        if purpose == Purpose::Open {
                            heading(ui, "recent");
                            if listed.recent.is_empty() {
                                muted(ui, "none yet");
                            }
                        }
                        for path in &listed.recent {
                            if recipe_row(ui, index == cursor, path, thumbs).clicked() {
                                clicked = Some(index);
                            }
                            index += 1;
                        }
                        for (title, recipes) in &listed.groups {
                            heading(ui, title);
                            if recipes.is_empty() {
                                muted(ui, "none here");
                                continue;
                            }
                            for path in recipes {
                                if recipe_row(ui, index == cursor, path, thumbs).clicked() {
                                    clicked = Some(index);
                                }
                                index += 1;
                            }
                        }
                    } else {
                        heading(ui, "matches");
                        if listed.matches.is_empty() {
                            muted(ui, "nothing matches");
                        }
                        for (index, row) in listed.matches.iter().enumerate() {
                            let response = match row {
                                Row::Recipe(path) => recipe_row(ui, index == cursor, path, thumbs),
                                Row::Dir(path) => dir_row(ui, index == cursor, path),
                            };
                            if response.clicked() {
                                clicked = Some(index);
                            }
                        }
                    }
                });
        }
        let (enter_clicked, escape_clicked) = footer(ui, live, purpose);
        let enter = enter || enter_clicked;
        if escape || escape_clicked {
            if live && self.shown {
                return Some(Act::Close);
            }
            self.query.clear();
            self.seen.clear();
            self.error = None;
            self.cursor = 0;
            self.shown = false;
            self.held = None;
            return None;
        }
        if clicked.is_none() && !enter {
            return None;
        }
        let index = clicked.unwrap_or(self.cursor);
        let chosen = self.held.as_ref().and_then(|held| held.listed.get(index));
        if purpose.saves() {
            return self.save_choice(clicked.is_some(), chosen);
        }
        match choose(&self.query, chosen.as_ref()) {
            Some(Choice::Open(path)) if purpose == Purpose::Import => Some(Act::Import(path)),
            Some(Choice::Open(path)) => Some(Act::Open(path)),
            Some(Choice::Browse(path)) => {
                self.browse_to(path);
                self.focus = true;
                None
            }
            None => {
                let query = self.query.trim();
                self.error = Some(if query.is_empty() {
                    "type a path".to_string()
                } else {
                    format!("nothing to {} at {query}", purpose.verb())
                });
                None
            }
        }
    }

    fn browse_to(&mut self, path: PathBuf) {
        let mut text = path.display().to_string();
        if !text.ends_with(std::path::is_separator) {
            text.push(std::path::MAIN_SEPARATOR);
        }
        self.anchor = Some(path);
        self.query = text;
        self.seen.clone_from(&self.query);
        self.cursor = 0;
        self.error = None;
    }

    fn save_choice(&mut self, clicked: bool, chosen: Option<Row>) -> Option<Act> {
        match chosen.filter(|_| clicked) {
            Some(Row::Dir(path)) => {
                self.browse_to(path);
                return None;
            }
            Some(Row::Recipe(path)) => {
                self.name = file_name(&path);
                if let Some(parent) = path.parent() {
                    self.browse_to(parent.to_path_buf());
                }
                return None;
            }
            None => {}
        }
        let name = self.name.trim();
        if name.is_empty() {
            self.error = Some("type a name".to_string());
            return None;
        }
        let query = self.query.trim();
        let folder = if query.is_empty() {
            self.anchor.clone().unwrap_or_else(|| PathBuf::from("."))
        } else {
            expand_home(query, home().as_deref())
        };
        if !folder.is_dir() {
            self.error = Some(format!("no folder at {}", folder.display()));
            return None;
        }
        let target = folder.join(name);
        Some(match self.purpose {
            Purpose::Export => Act::Export(target, self.selected.unwrap_or(false)),
            _ => Act::SaveAs(target),
        })
    }
}

fn title(ui: &mut Ui, purpose: Purpose) {
    let palette = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(purpose.title())
                .font(Theme::of(ui.ctx()).bold_font(SIZE_BODY))
                .color(palette.text_secondary),
        );
        let keys = match purpose {
            Purpose::Open => Some("ctrl o"),
            Purpose::SaveAs => Some("ctrl ⇧ s"),
            Purpose::Import | Purpose::Export => None,
        };
        if let Some(keys) = keys {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(small(ui, purpose.verb(), palette.text_secondary));
                ui.add(KeyCap::new(keys));
            });
        }
    });
}

fn heading(ui: &mut Ui, title: &str) {
    let palette = Palette::of(ui.ctx());
    ui.add_space(4.0);
    ui.label(small(ui, title.to_uppercase(), palette.text_secondary));
}

fn muted(ui: &mut Ui, line: &str) {
    let palette = Palette::of(ui.ctx());
    ui.label(code(ui, line, palette.text_muted));
}

fn footer(ui: &mut Ui, live: bool, purpose: Purpose) -> (bool, bool) {
    let palette = Palette::of(ui.ctx());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let enter = ui.add(KeyCap::new("enter").button()).clicked();
        ui.label(small(ui, purpose.verb(), palette.text_secondary));
        ui.add_space(8.0);
        let escape = ui.add(KeyCap::new("esc").button()).clicked();
        ui.label(small(
            ui,
            if live { "close" } else { "clear" },
            palette.text_secondary,
        ));
        ui.add_space(8.0);
        ui.add(KeyCap::new("↑↓"));
        ui.label(small(ui, "choose", palette.text_secondary));
        (enter, escape)
    })
    .inner
}

fn recipe_row(ui: &mut Ui, selected: bool, path: &Path, thumbs: &mut Thumbs) -> egui::Response {
    let palette = Palette::of(ui.ctx());
    let name = file_name(path);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(crate::shown::path)
        .unwrap_or_default();
    let thumb = thumbs.get(ui.ctx(), path);
    let icon = if thumb.is_some() { "" } else { FILE };
    let response = entry(
        ui,
        selected,
        (icon, palette.info),
        &name,
        &detail(path, &parent),
    );
    if let Some(texture) = thumb {
        let side = response.rect.height() - 4.0;
        let at = egui::Rect::from_min_size(
            response.rect.left_top() + Vec2::new(2.0, 2.0),
            Vec2::splat(side),
        );
        ui.painter().image(
            texture,
            at,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    response
}

pub fn detail(path: &Path, parent: &str) -> String {
    match crate::file::kind(path) {
        Some(crate::file::Kind::Scene) => format!("scene  {}", tail_text(parent, 35)),
        Some(crate::file::Kind::Pfx) => format!("pfx  {}", tail_text(parent, 36)),
        Some(crate::file::Kind::Glb) => format!("glb  {}", tail_text(parent, 37)),
        _ => tail_text(parent, 42),
    }
}

fn dir_row(ui: &mut Ui, selected: bool, path: &Path) -> egui::Response {
    let palette = Palette::of(ui.ctx());
    entry(
        ui,
        selected,
        (FOLDER, palette.text_muted),
        &file_name(path),
        "",
    )
}

fn entry(
    ui: &mut Ui,
    selected: bool,
    icon: (&str, Color32),
    label: &str,
    detail: &str,
) -> egui::Response {
    let response = list_row(ui, true, selected, 0, icon, label, detail);
    if selected {
        response.scroll_to_me(None);
    }
    response
}

fn pressed(ui: &Ui, key: Key) -> bool {
    ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: found,
                    pressed: true,
                    modifiers,
                    ..
                } if *found == key && modifiers.is_none()
            )
        })
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn visible_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if name.starts_with('.') {
        None
    } else {
        Some(name)
    }
}

fn by_name(a: &Path, b: &Path) -> std::cmp::Ordering {
    a.file_name().cmp(&b.file_name()).then(a.cmp(b))
}

fn expand_home(typed: &str, home: Option<&OsStr>) -> PathBuf {
    match (
        typed
            .strip_prefix('~')
            .and_then(|rest| rest.strip_prefix(std::path::is_separator)),
        home,
    ) {
        (Some(rest), Some(home)) if !home.is_empty() => PathBuf::from(home).join(rest),
        _ => PathBuf::from(typed),
    }
}

fn split_query(typed: &str) -> (PathBuf, String) {
    split_query_with(typed, home().as_deref())
}

fn split_query_with(typed: &str, home: Option<&OsStr>) -> (PathBuf, String) {
    let expanded = expand_home(typed, home);
    if typed.ends_with(std::path::is_separator) {
        return (expanded, String::new());
    }
    let path = expanded.as_path();
    let prefix = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    (dir, prefix)
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "recent".to_string());
    let temp = path.with_file_name(format!(".{name}.pitofx-tmp"));
    std::fs::write(&temp, bytes)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let folder = crate::scratch(name).parent().unwrap().to_path_buf();
        std::fs::remove_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|path| file_name(path)).collect()
    }

    #[test]
    fn the_picker_lists_a_folders_recipes_sorted_by_name() {
        let folder = dir("open-list");
        std::fs::write(folder.join("c.toml"), "c").unwrap();
        std::fs::write(folder.join("a.toml"), "a").unwrap();
        std::fs::write(folder.join("b.toml"), "b").unwrap();
        std::fs::write(folder.join("notes.txt"), "n").unwrap();
        std::fs::write(folder.join(".hidden.toml"), "h").unwrap();
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join("sub/d.toml"), "d").unwrap();
        assert_eq!(names(&recipes(&folder)), ["a.toml", "b.toml", "c.toml"]);
        let listed = listed("", &[], &[("scenes".into(), folder.clone())]);
        assert!(listed.recent.is_empty());
        assert_eq!(listed.groups.len(), 1);
        assert_eq!(names(&listed.groups[0].1), ["a.toml", "b.toml", "c.toml"]);
        assert_eq!(listed.len(), 3);
        assert_eq!(
            listed.get(0).and_then(|row| match row {
                Row::Recipe(path) => Some(file_name(&path)),
                Row::Dir(_) => None,
            }),
            Some("a.toml".to_string())
        );
        assert_eq!(listed.get(3), None);
    }

    #[test]
    fn the_picker_lists_scene_files_beside_recipes_and_marks_them() {
        let folder = dir("open-list-scene");
        std::fs::write(folder.join("room.scene.toml"), "s").unwrap();
        std::fs::write(folder.join("desk.toml"), "d").unwrap();
        let found = recipes(&folder);
        assert_eq!(names(&found), ["desk.toml", "room.scene.toml"]);
        assert_eq!(detail(&found[1], "scenes"), "scene  scenes");
        assert_eq!(detail(&found[0], "scenes"), "scenes");
    }

    #[test]
    fn recent_recipes_are_kept_deduplicated_and_capped_at_ten() {
        let folder = dir("open-recent");
        let book = Book::at(&folder.join("state"));
        let mut made = Vec::new();
        for index in 0..12 {
            let path = folder.join(format!("r{index}.toml"));
            std::fs::write(&path, "recipe = 1\n").unwrap();
            book.remember(&path).unwrap();
            made.push(std::path::absolute(&path).unwrap());
        }
        let list = book.list();
        assert_eq!(list.len(), LIMIT);
        assert_eq!(list[0], made[11]);
        assert!(!list.contains(&made[0]));
        assert!(!list.contains(&made[1]));
        book.remember(&made[5]).unwrap();
        let list = book.list();
        assert_eq!(list.len(), LIMIT);
        assert_eq!(list[0], made[5]);
        assert_eq!(list.iter().filter(|path| *path == &made[5]).count(), 1);
        assert_eq!(list[1], made[11]);
        let held = std::fs::read_dir(folder.join("state"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(held, [SETTINGS]);
        let reader = pfx_file::Reader::open(book.path.as_ref().unwrap()).unwrap();
        let text = reader
            .named(pfx_file::FILE, SETTINGS_TOML)
            .unwrap()
            .unwrap();
        let table: toml_edit::DocumentMut = std::str::from_utf8(&text).unwrap().parse().unwrap();
        assert_eq!(table["recent"].as_array().unwrap().len(), LIMIT);
        book.keep_view(crate::wire::Look {
            xray: true,
            ..crate::wire::Look::default()
        })
        .unwrap();
        assert!(book.view().xray);
        assert_eq!(book.list().len(), LIMIT);
    }

    #[test]
    fn the_path_field_completes_files_and_folders() {
        let folder = dir("open-complete");
        std::fs::write(folder.join("toybox.toml"), "s").unwrap();
        std::fs::write(folder.join("table.toml"), "p").unwrap();
        std::fs::write(folder.join("readme.txt"), "r").unwrap();
        std::fs::create_dir_all(folder.join("more")).unwrap();
        let prefix = format!("{}/to", folder.display());
        assert_eq!(names(&complete(&prefix)), ["toybox.toml"]);
        let inside = format!("{}/", folder.display());
        assert_eq!(
            names(&complete(&inside)),
            ["more", "table.toml", "toybox.toml"]
        );
        let listed = listed(&prefix, &[], &[]);
        assert_eq!(
            listed.matches,
            vec![Row::Recipe(folder.join("toybox.toml"))]
        );
    }

    #[test]
    fn a_leading_tilde_slash_is_the_home_folder_in_choose_and_completion() {
        let home = dir("open-home");
        std::fs::create_dir_all(home.join("scenes")).unwrap();
        std::fs::write(home.join("scenes/toybox.toml"), "recipe = 1\n").unwrap();
        std::fs::write(home.join("scenes/table.toml"), "recipe = 1\n").unwrap();
        let root = home.clone();
        let home = Some(home.as_os_str());
        assert_eq!(
            choose_with("~/scenes/toybox.toml", None, home),
            Some(Choice::Open(root.join("scenes/toybox.toml")))
        );
        assert_eq!(
            choose_with("~/scenes/", None, home),
            Some(Choice::Browse(root.join("scenes/")))
        );
        assert_eq!(choose_with("~/scenes/missing.toml", None, home), None);
        let (folder, prefix) = split_query_with("~/scenes/to", home);
        assert_eq!(folder, root.join("scenes"));
        assert_eq!(prefix, "to");
        let (folder, prefix) = split_query_with("~/scenes/", home);
        assert_eq!(folder, root.join("scenes/"));
        assert_eq!(prefix, "");
        assert_eq!(
            split_query_with("~/scenes/to", None),
            (PathBuf::from("~/scenes"), "to".to_string())
        );
        assert_eq!(
            split_query_with("~x/scenes", home),
            (PathBuf::from("~x"), "scenes".to_string())
        );
    }

    #[test]
    fn enter_opens_a_typed_file_and_browses_a_typed_folder() {
        let folder = dir("open-choose");
        let recipe = folder.join("scene.toml");
        std::fs::write(&recipe, "recipe = 1\n").unwrap();
        assert_eq!(
            choose(&recipe.display().to_string(), None),
            Some(Choice::Open(recipe))
        );
        assert_eq!(
            choose(&folder.display().to_string(), None),
            Some(Choice::Browse(folder.clone()))
        );
        let selected = Row::Recipe(folder.join("other.toml"));
        assert_eq!(
            choose("", Some(&selected)),
            Some(Choice::Open(folder.join("other.toml")))
        );
        assert_eq!(choose("missing.toml", None), None);
    }

    #[test]
    fn state_home_is_the_recent_folder_and_home_is_the_fallback() {
        let xdg = OsStr::new("/var/state");
        let home = OsStr::new("/u/dad");
        assert_eq!(
            state_dir_from(Some(xdg), Some(home)),
            PathBuf::from("/var/state/pfx")
        );
        assert_eq!(
            state_dir_from(None, Some(home)),
            PathBuf::from("/u/dad/.local/state/pfx")
        );
        assert_eq!(
            state_dir_from(Some(OsStr::new("")), Some(home)),
            PathBuf::from("/u/dad/.local/state/pfx")
        );
    }

    #[test]
    fn samples_is_the_folder_beside_the_repo_when_it_is_there() {
        let root = dir("open-walk");
        let samples = root.join("samples");
        std::fs::create_dir_all(samples.join("nested")).unwrap();
        let start = root.join("target/debug/deps");
        std::fs::create_dir_all(&start).unwrap();
        assert_eq!(samples_from(&start), Some(samples));
        let lost = dir("open-walk-lost").join("a/b/c/d/e/f/g/h/i");
        std::fs::create_dir_all(&lost).unwrap();
        assert_eq!(samples_from(&lost), None);
    }

    #[test]
    fn a_binary_outside_the_tree_lists_the_installed_samples() {
        let outside = dir("open-bin").join("a/b/c/d/e/f/g/h/i");
        std::fs::create_dir_all(&outside).unwrap();
        let installed = dir("open-data").join("pfx/samples");
        std::fs::create_dir_all(&installed).unwrap();
        assert_eq!(samples_with(Some(&outside), None), None);
        assert_eq!(
            samples_with(Some(&outside), Some(&installed)),
            Some(installed.clone())
        );
        assert_eq!(samples_with(None, Some(&installed)), Some(installed));
        assert_eq!(
            samples_with(Some(&outside), Some(&outside.join("missing"))),
            None
        );
        let tree = dir("open-tree");
        let beside = tree.join("samples");
        std::fs::create_dir_all(&beside).unwrap();
        let start = tree.join("target/debug");
        std::fs::create_dir_all(&start).unwrap();
        assert_eq!(samples_with(Some(&start), Some(&outside)), Some(beside));
    }

    #[test]
    fn the_data_folder_follows_xdg_data_home_then_home() {
        let xdg = OsStr::new("/var/share");
        let home = OsStr::new("/u/dad");
        assert_eq!(
            data_dir_from(Some(xdg), Some(home)),
            PathBuf::from("/var/share/pfx")
        );
        assert_eq!(
            data_dir_from(None, Some(home)),
            PathBuf::from("/u/dad/.local/share/pfx")
        );
        assert_eq!(
            data_dir_from(Some(OsStr::new("")), Some(home)),
            PathBuf::from("/u/dad/.local/share/pfx")
        );
    }

    #[test]
    fn known_folders_keep_samples_once() {
        let root = dir("open-known");
        let samples = root.join("samples");
        let scenes = root.join("scenes");
        std::fs::create_dir_all(&samples).unwrap();
        std::fs::create_dir_all(&scenes).unwrap();
        let folders = known(Some(&samples), Some(&scenes), Some(&samples));
        assert_eq!(
            folders,
            vec![
                ("samples".to_string(), samples),
                ("scenes".to_string(), scenes),
            ]
        );
    }
}
