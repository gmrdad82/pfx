pub const REMEMBERED: usize = 512;

pub fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub len: u64,
    pub hash: u64,
}

impl Stamp {
    pub fn of(text: &str) -> Stamp {
        Stamp {
            len: text.len() as u64,
            hash: hash(text),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Current,
    Earlier,
    Outside,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disk {
    Same,
    Own,
    Outside(String),
    Missing,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Own {
    current: Option<Stamp>,
    written: Vec<Stamp>,
}

impl Own {
    pub fn read(&mut self, text: &str) {
        self.current = Some(Stamp::of(text));
    }

    pub fn wrote(&mut self, text: &str) -> Stamp {
        let stamp = Stamp::of(text);
        self.current = Some(stamp);
        self.written.retain(|known| *known != stamp);
        self.written.push(stamp);
        if self.written.len() > REMEMBERED {
            self.written.remove(0);
        }
        stamp
    }

    pub fn current(&self) -> Option<Stamp> {
        self.current
    }

    pub fn seen(&self, stamp: Stamp) -> Seen {
        if self.current == Some(stamp) {
            Seen::Current
        } else if self.written.contains(&stamp) {
            Seen::Earlier
        } else {
            Seen::Outside
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_is_the_length_and_the_hash() {
        assert_eq!(Stamp::of(""), Stamp::of(""));
        assert_ne!(Stamp::of("a = 1\n"), Stamp::of("a = 2\n"));
        assert_eq!(Stamp::of("abc").len, 3);
        assert_eq!(hash(""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn own_writes_are_told_from_outside_ones() {
        let mut own = Own::default();
        own.read("a = 1\n");
        assert_eq!(own.seen(Stamp::of("a = 1\n")), Seen::Current);
        own.wrote("a = 2\n");
        own.wrote("a = 3\n");
        assert_eq!(own.seen(Stamp::of("a = 3\n")), Seen::Current);
        assert_eq!(own.seen(Stamp::of("a = 2\n")), Seen::Earlier);
        assert_eq!(own.seen(Stamp::of("a = 1\n")), Seen::Outside);
        assert_eq!(own.seen(Stamp::of("a = 4\n")), Seen::Outside);
    }

    #[test]
    fn only_the_last_writes_are_remembered() {
        let mut own = Own::default();
        for step in 0..REMEMBERED + 2 {
            own.wrote(&step.to_string());
        }
        assert_eq!(own.seen(Stamp::of("0")), Seen::Outside);
        assert_eq!(own.seen(Stamp::of("2")), Seen::Earlier);
    }
}
