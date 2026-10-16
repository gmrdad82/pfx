pub trait Clipboard {
    fn set_text(&self, text: &str) -> bool;
    fn get_text(&self) -> Option<String>;
    fn refresh(&self) {}
}
