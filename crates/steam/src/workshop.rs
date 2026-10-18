use std::path::PathBuf;

use crate::Error;

pub const TITLE_MAX: usize = 128;
pub const DESCRIPTION_MAX: usize = 8000;
pub const TAG_MAX: usize = 255;
pub const NOTE_MAX: usize = 8000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Item(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemState {
    pub subscribed: bool,
    pub installed: bool,
    pub needs_update: bool,
    pub downloading: bool,
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub folder: PathBuf,
    pub bytes: u64,
    pub updated: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Created {
    pub item: Item,
    pub agreement: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Friends,
    #[default]
    Private,
    Unlisted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Config,
    Content,
    Uploading,
    Preview,
    Committing,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub content: Option<PathBuf>,
    pub preview: Option<PathBuf>,
    pub visibility: Option<Visibility>,
    pub tags: Option<Vec<String>>,
    pub note: Option<String>,
}

impl ItemUpdate {
    pub fn check(&self) -> Result<(), Error> {
        let text = |text: &Option<String>, max: usize| match text {
            Some(text) if text.len() > max || text.as_bytes().contains(&0) => Err(Error::Rejected),
            _ => Ok(()),
        };
        text(&self.title, TITLE_MAX)?;
        text(&self.description, DESCRIPTION_MAX)?;
        text(&self.note, NOTE_MAX)?;
        for tag in self.tags.iter().flatten() {
            if tag.is_empty()
                || tag.len() > TAG_MAX
                || tag.contains(',')
                || !tag.chars().all(|c| c.is_ascii_graphic() || c == ' ')
            {
                return Err(Error::Rejected);
            }
        }
        if let Some(content) = &self.content
            && !content.is_dir()
        {
            return Err(Error::Missing);
        }
        if let Some(preview) = &self.preview
            && !preview.is_file()
        {
            return Err(Error::Missing);
        }
        Ok(())
    }
}

#[cfg(any(feature = "mods", test))]
pub use mods::{ItemRefusal, Items, load_items};

#[cfg(any(feature = "mods", test))]
mod mods {
    use std::collections::BTreeMap;
    use std::fmt;

    use pfx_mod::{Host, LoadError, Package};

    use super::Item;
    use crate::{Error, SteamApi};

    pub struct Items<T: 'static> {
        pub packages: Vec<(Item, Package<T>)>,
        pub refused: Vec<(Item, ItemRefusal)>,
        pub waiting: Vec<Item>,
        pub updating: Vec<Item>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ItemRefusal {
        Load(LoadError),
        Taken { id: String, by: Option<Item> },
        Steam(Error),
    }

    impl fmt::Display for ItemRefusal {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Load(error) => write!(f, "{error}"),
                Self::Taken { id, by: Some(by) } => write!(
                    f,
                    "holds mod `{id}`, which Workshop item {} already brings; the first one loads",
                    by.0
                ),
                Self::Taken { id, by: None } => write!(
                    f,
                    "holds mod `{id}`, which a local mod already brings; the local one loads"
                ),
                Self::Steam(error) => write!(f, "cannot be read: {error}"),
            }
        }
    }

    pub fn load_items<T: 'static>(
        steam: &dyn SteamApi,
        host: &Host<T>,
        local: &[Package<T>],
    ) -> Result<Items<T>, Error> {
        let mut subscribed = steam.subscribed_items()?;
        subscribed.sort();
        subscribed.dedup();
        let mut taken: BTreeMap<String, Option<Item>> = local
            .iter()
            .map(|package| (package.manifest.id.clone(), None))
            .collect();
        let mut items = Items {
            packages: Vec::new(),
            refused: Vec::new(),
            waiting: Vec::new(),
            updating: Vec::new(),
        };
        for item in subscribed {
            let state = match steam.item_state(item) {
                Ok(state) => state,
                Err(error) => {
                    items.refused.push((item, ItemRefusal::Steam(error)));
                    continue;
                }
            };
            let installed = match steam.item_installed(item) {
                Ok(installed) => installed.filter(|_| state.installed),
                Err(error) => {
                    items.refused.push((item, ItemRefusal::Steam(error)));
                    continue;
                }
            };
            let Some(installed) = installed else {
                items.waiting.push(item);
                continue;
            };
            if state.needs_update || state.downloading || state.pending {
                items.updating.push(item);
            }
            let package = match host.load_package(&installed.folder) {
                Ok(package) => package,
                Err(error) => {
                    items.refused.push((item, ItemRefusal::Load(error)));
                    continue;
                }
            };
            if let Some(by) = taken.get(&package.manifest.id) {
                items.refused.push((
                    item,
                    ItemRefusal::Taken {
                        id: package.manifest.id.clone(),
                        by: *by,
                    },
                ));
                continue;
            }
            taken.insert(package.manifest.id.clone(), Some(item));
            items.packages.push((item, package));
        }
        Ok(items)
    }
}
