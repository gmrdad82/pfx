use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Data {
    files: Arc<[(String, Arc<[u8]>)]>,
}

impl Data {
    pub(crate) fn new(files: Vec<(String, Arc<[u8]>)>) -> Self {
        Self {
            files: files.into(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(file, _)| file == name)
            .map(|(_, bytes)| &bytes[..])
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(name, bytes)| (name.as_str(), &bytes[..]))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(name, _)| name.as_str())
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|(_, bytes)| bytes.len() as u64).sum()
    }
}
