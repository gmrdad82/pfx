use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use pfx_play::Scheme;

const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";
const CHANGED: &str = "SettingChanged";
const READ_TIMEOUT: Duration = Duration::from_millis(500);
const WATCH_TIMEOUT: Duration = Duration::from_secs(5);
const LARGEST: usize = 1 << 20;

const CALL: u8 = 1;
const ERROR: u8 = 3;
const SIGNAL: u8 = 4;
const RETURN: u8 = 2;

pub(super) fn read() -> Option<Scheme> {
    Bus::open(READ_TIMEOUT)?.scheme().ok().flatten()
}

pub(super) fn watch(send: impl Fn(Option<Scheme>) -> bool + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name("pfx scheme".into())
        .spawn(move || {
            let Some(mut bus) = Bus::open(WATCH_TIMEOUT) else {
                send(None);
                return;
            };
            let rule = format!(
                "type='signal',interface='{SETTINGS}',member='{CHANGED}',path='{PORTAL_PATH}',arg0='{NAMESPACE}',arg1='{KEY}'"
            );
            let matched = bus
                .call(BUS, BUS_PATH, BUS, "AddMatch", "s", &[&rule])
                .and_then(|serial| bus.reply(serial));
            let first = bus.scheme().ok().flatten();
            if !send(first) || matched.is_err() {
                return;
            }
            if bus.stream.set_read_timeout(None).is_err() {
                return;
            }
            while let Ok(message) = bus.message() {
                if message.kind == SIGNAL
                    && message.member.as_deref() == Some(CHANGED)
                    && let Some(value) = message.setting()
                    && !send(scheme_of(value))
                {
                    return;
                }
            }
        })
        .is_ok()
}

fn scheme_of(value: u32) -> Option<Scheme> {
    match value {
        1 => Some(Scheme::Dark),
        2 => Some(Scheme::Light),
        _ => None,
    }
}

struct Bus {
    stream: UnixStream,
    serial: u32,
}

struct Message {
    kind: u8,
    little: bool,
    reply: Option<u32>,
    member: Option<String>,
    signature: String,
    body: Vec<u8>,
}

impl Bus {
    fn open(timeout: Duration) -> Option<Self> {
        let stream = connect()?;
        stream.set_read_timeout(Some(timeout)).ok()?;
        stream.set_write_timeout(Some(timeout)).ok()?;
        let mut bus = Self { stream, serial: 0 };
        bus.authenticate().ok()?;
        let serial = bus.call(BUS, BUS_PATH, BUS, "Hello", "", &[]).ok()?;
        bus.reply(serial).ok()?;
        Some(bus)
    }

    fn authenticate(&mut self) -> io::Result<()> {
        self.stream.write_all(b"\0AUTH EXTERNAL\r\n")?;
        let mut line = self.line()?;
        if line.starts_with("DATA") {
            self.stream.write_all(b"DATA\r\n")?;
            line = self.line()?;
        }
        if !line.starts_with("OK") {
            return Err(io::Error::other(format!("the bus refused: {line}")));
        }
        self.stream.write_all(b"BEGIN\r\n")
    }

    fn line(&mut self) -> io::Result<String> {
        let mut line = Vec::new();
        let mut byte = [0u8];
        while !line.ends_with(b"\r\n") {
            self.stream.read_exact(&mut byte)?;
            line.push(byte[0]);
            if line.len() > 512 {
                return Err(io::Error::other("the bus sent too long a line"));
            }
        }
        Ok(String::from_utf8_lossy(&line).trim().to_string())
    }

    fn scheme(&mut self) -> io::Result<Option<Scheme>> {
        for member in ["ReadOne", "Read"] {
            let serial = self.call(
                PORTAL,
                PORTAL_PATH,
                SETTINGS,
                member,
                "ss",
                &[NAMESPACE, KEY],
            )?;
            let reply = self.reply(serial)?;
            if reply.kind == RETURN {
                return Ok(reply.value().and_then(scheme_of));
            }
        }
        Ok(None)
    }

    fn call(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        args: &[&str],
    ) -> io::Result<u32> {
        self.serial += 1;
        let mut body = Vec::new();
        for arg in args {
            string(&mut body, arg);
        }
        let mut fields = Vec::new();
        field(&mut fields, 1, 'o', path);
        field(&mut fields, 2, 's', interface);
        field(&mut fields, 3, 's', member);
        field(&mut fields, 6, 's', destination);
        if !signature.is_empty() {
            field(&mut fields, 8, 'g', signature);
        }
        let mut message = vec![b'l', CALL, 0, 1];
        message.extend_from_slice(&(body.len() as u32).to_le_bytes());
        message.extend_from_slice(&self.serial.to_le_bytes());
        message.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        message.extend_from_slice(&fields);
        pad(&mut message, 8);
        message.extend_from_slice(&body);
        self.stream.write_all(&message)?;
        Ok(self.serial)
    }

    fn reply(&mut self, serial: u32) -> io::Result<Message> {
        loop {
            let message = self.message()?;
            if matches!(message.kind, RETURN | ERROR) && message.reply == Some(serial) {
                return Ok(message);
            }
        }
    }

    fn message(&mut self) -> io::Result<Message> {
        let mut head = [0u8; 16];
        self.stream.read_exact(&mut head)?;
        let little = head[0] == b'l';
        let word = |bytes: &[u8]| {
            let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
            if little {
                u32::from_le_bytes(bytes)
            } else {
                u32::from_be_bytes(bytes)
            }
        };
        let body_length = word(&head[4..8]) as usize;
        let fields_length = word(&head[12..16]) as usize;
        if body_length + fields_length > LARGEST {
            return Err(io::Error::other("the bus sent too large a message"));
        }
        let header = (16 + fields_length).next_multiple_of(8);
        let mut rest = vec![0u8; header - 16 + body_length];
        self.stream.read_exact(&mut rest)?;
        let mut message = Message {
            kind: head[1],
            little,
            reply: None,
            member: None,
            signature: String::new(),
            body: rest.split_off(header - 16),
        };
        let mut fields = Reader {
            data: &rest[..fields_length],
            at: 0,
            base: 16,
            little,
        };
        while fields.at < fields.data.len() {
            fields.align(8)?;
            let code = fields.byte()?;
            let signature = fields.signature()?;
            match (code, signature.as_str()) {
                (3, "s") => message.member = Some(fields.string()?),
                (5, "u") => message.reply = Some(fields.word()?),
                (8, "g") => message.signature = fields.signature()?,
                (_, signature) => fields.skip(signature)?,
            }
        }
        Ok(message)
    }
}

impl Message {
    fn reader(&self) -> Reader<'_> {
        Reader {
            data: &self.body,
            at: 0,
            base: 0,
            little: self.little,
        }
    }

    fn value(&self) -> Option<u32> {
        (self.signature == "v").then_some(())?;
        self.reader().variant().ok()
    }

    fn setting(&self) -> Option<u32> {
        (self.signature == "ssv").then_some(())?;
        let mut reader = self.reader();
        let namespace = reader.string().ok()?;
        let key = reader.string().ok()?;
        (namespace == NAMESPACE && key == KEY).then_some(())?;
        reader.variant().ok()
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    base: usize,
    little: bool,
}

fn short() -> io::Error {
    io::Error::other("the bus sent a short message")
}

impl Reader<'_> {
    fn align(&mut self, to: usize) -> io::Result<()> {
        let at = (self.base + self.at).next_multiple_of(to) - self.base;
        if at > self.data.len() {
            return Err(short());
        }
        self.at = at;
        Ok(())
    }

    fn take(&mut self, count: usize) -> io::Result<&[u8]> {
        let end = self.at.checked_add(count).ok_or_else(short)?;
        let bytes = self.data.get(self.at..end).ok_or_else(short)?;
        self.at = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn word(&mut self) -> io::Result<u32> {
        self.align(4)?;
        let little = self.little;
        let bytes = self.take(4)?;
        let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
        Ok(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    fn string(&mut self) -> io::Result<String> {
        let length = self.word()? as usize;
        let text = String::from_utf8_lossy(self.take(length)?).into_owned();
        self.take(1)?;
        Ok(text)
    }

    fn signature(&mut self) -> io::Result<String> {
        let length = usize::from(self.byte()?);
        let text = String::from_utf8_lossy(self.take(length)?).into_owned();
        self.take(1)?;
        Ok(text)
    }

    fn skip(&mut self, signature: &str) -> io::Result<()> {
        match signature {
            "s" | "o" => self.string().map(drop),
            "g" => self.signature().map(drop),
            "u" | "i" | "b" => self.word().map(drop),
            "y" => self.byte().map(drop),
            _ => Err(io::Error::other("the bus sent an unknown header field")),
        }
    }

    fn variant(&mut self) -> io::Result<u32> {
        match self.signature()?.as_str() {
            "u" | "i" => self.word(),
            "v" => self.variant(),
            _ => Err(io::Error::other("the setting is not a number")),
        }
    }
}

fn pad(buffer: &mut Vec<u8>, to: usize) {
    buffer.resize(buffer.len().next_multiple_of(to), 0);
}

fn string(buffer: &mut Vec<u8>, text: &str) {
    pad(buffer, 4);
    buffer.extend_from_slice(&(text.len() as u32).to_le_bytes());
    buffer.extend_from_slice(text.as_bytes());
    buffer.push(0);
}

fn field(buffer: &mut Vec<u8>, code: u8, kind: char, value: &str) {
    pad(buffer, 8);
    buffer.extend_from_slice(&[code, 1, kind as u8, 0]);
    if kind == 'g' {
        buffer.push(value.len() as u8);
        buffer.extend_from_slice(value.as_bytes());
        buffer.push(0);
    } else {
        string(buffer, value);
    }
}

fn unescape(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    out
}

fn connect() -> Option<UnixStream> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    for entry in address.split(';') {
        let Some(keys) = entry.strip_prefix("unix:") else {
            continue;
        };
        for pair in keys.split(',') {
            if let Some(path) = pair.strip_prefix("path=") {
                use std::os::unix::ffi::OsStringExt;
                let path = PathBuf::from(std::ffi::OsString::from_vec(unescape(path)));
                if let Ok(stream) = UnixStream::connect(path) {
                    return Some(stream);
                }
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            if let Some(name) = pair.strip_prefix("abstract=") {
                #[cfg(target_os = "android")]
                use std::os::android::net::SocketAddrExt;
                #[cfg(target_os = "linux")]
                use std::os::linux::net::SocketAddrExt;
                if let Ok(address) =
                    std::os::unix::net::SocketAddr::from_abstract_name(unescape(name))
                    && let Ok(stream) = UnixStream::connect_addr(&address)
                {
                    return Some(stream);
                }
            }
        }
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    UnixStream::connect(PathBuf::from(runtime).join("bus")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(signature: &str, body: Vec<u8>, little: bool) -> Message {
        Message {
            kind: RETURN,
            little,
            reply: Some(1),
            member: None,
            signature: signature.into(),
            body,
        }
    }

    #[test]
    fn settings_read_from_both_portal_replies_and_the_change_signal() {
        let read_one = vec![1, b'u', 0, 0, 1, 0, 0, 0];
        assert_eq!(reply("v", read_one, true).value(), Some(1));
        let read = vec![1, b'v', 0, 1, b'u', 0, 0, 0, 0, 0, 0, 2];
        assert_eq!(reply("v", read, false).value(), Some(2));
        let mut changed = Vec::new();
        string(&mut changed, NAMESPACE);
        string(&mut changed, KEY);
        changed.extend_from_slice(&[1, b'u', 0]);
        pad(&mut changed, 4);
        changed.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(reply("ssv", changed.clone(), true).setting(), Some(1));
        let mut other = Vec::new();
        string(&mut other, NAMESPACE);
        string(&mut other, "accent-color");
        other.extend_from_slice(&changed[changed.len() - 8..]);
        assert_eq!(reply("ssv", other, true).setting(), None);
        assert_eq!(scheme_of(0), None);
        assert_eq!(scheme_of(1), Some(Scheme::Dark));
        assert_eq!(scheme_of(2), Some(Scheme::Light));
        assert_eq!(unescape("/run/user/1000%2fbus"), b"/run/user/1000/bus");
    }
}
