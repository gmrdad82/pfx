use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

pub const URL_LIMIT: usize = 4096;

const REFUSED: &[u8] = b"\\\"<>`{}|^";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlError {
    Scheme,
    Malformed(&'static str),
    NoBrowser,
    Busy,
    Failed(String),
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scheme => write!(f, "only http and https links open"),
            Self::Malformed(why) => write!(f, "the link is refused: {why}"),
            Self::NoBrowser => write!(f, "no browser is attached to this run"),
            Self::Busy => write!(f, "another link is still opening"),
            Self::Failed(why) => write!(f, "the link did not open: {why}"),
        }
    }
}

impl std::error::Error for UrlError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Url(String);

impl Url {
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        if text.len() > URL_LIMIT {
            return Err(UrlError::Malformed("longer than 4096 bytes"));
        }
        let Some((scheme, rest)) = text.split_once("://") else {
            return Err(UrlError::Scheme);
        };
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err(UrlError::Scheme);
        }
        if !text
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !REFUSED.contains(&byte))
        {
            return Err(UrlError::Malformed(
                "a space, a control character, one of \\ \" < > ` { } | ^, or a character outside ASCII",
            ));
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.contains('@') {
            return Err(UrlError::Malformed("a user name before the host"));
        }
        if authority.is_empty() || authority.starts_with(':') {
            return Err(UrlError::Malformed("no host"));
        }
        let path = &rest[authority.len()..];
        Ok(Self(format!(
            "{}://{}{path}",
            scheme.to_ascii_lowercase(),
            authority.to_ascii_lowercase()
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Answer = Sender<Result<(), UrlError>>;

#[derive(Debug)]
pub struct Opening {
    answer: Option<Receiver<Result<(), UrlError>>>,
    result: Option<Result<(), UrlError>>,
}

impl Opening {
    pub fn pending() -> (Answer, Self) {
        let (answer, receiver) = mpsc::channel();
        (
            answer,
            Self {
                answer: Some(receiver),
                result: None,
            },
        )
    }

    pub fn done(result: Result<(), UrlError>) -> Self {
        Self {
            answer: None,
            result: Some(result),
        }
    }

    pub fn poll(&mut self) -> Option<&Result<(), UrlError>> {
        if self.result.is_none()
            && let Some(answer) = &self.answer
        {
            self.result = match answer.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err(UrlError::Failed("the browser gave no answer".into())))
                }
            };
            if self.result.is_some() {
                self.answer = None;
            }
        }
        self.result.as_ref()
    }
}

pub trait Browser {
    fn open(&self, url: &Url) -> Opening;
}
