use crate::User;

pub const MESSAGE_MAX: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Reliable,
    Unreliable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub peer: User,
    pub channel: u32,
    pub bytes: Vec<u8>,
}
