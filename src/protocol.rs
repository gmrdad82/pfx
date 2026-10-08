use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Hello,
    Shots,
    Open {
        shot: String,
        fixtures: String,
        scale: f32,
        #[serde(default)]
        app: Value,
    },
    Input {
        event: Event,
    },
    Advance {
        to: f64,
    },
    Sample {
        spp: u32,
        #[serde(default)]
        moving: u32,
        seed: u64,
    },
    Frame {
        path: String,
        #[serde(default)]
        ground: Option<[f32; 3]>,
        #[serde(default)]
        linear: bool,
    },
    Audio {
        path: String,
        seconds: f64,
    },
    Close,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Event {
    Key {
        key: String,
        #[serde(default)]
        state: Option<String>,
    },
    Pointer {
        at: [f32; 2],
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        state: Option<String>,
    },
    Text {
        text: String,
    },
    Cue {
        cue: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Reply {
    Ok(Value),
    Err(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Hello {
    pub protocol: u32,
    pub app: String,
    pub rev: String,
    pub depth: u8,
    pub cutouts: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub audio_format: Option<AudioFormat>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    S16,
    F32,
}

impl AudioFormat {
    pub fn name(self) -> &'static str {
        match self {
            AudioFormat::S16 => "s16",
            AudioFormat::F32 => "f32",
        }
    }
}

pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Sounded {
    pub frames: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Opened {
    pub size: [u32; 2],
    pub gpu: String,
    pub driver: String,
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Written {
    pub size: [u32; 2],
    #[serde(default)]
    pub ground: Option<[f32; 3]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_read_as_the_design_writes_them() {
        let r: Request =
            serde_json::from_str(r#"{"op":"sample","spp":16,"moving":24,"seed":7}"#).unwrap();
        assert_eq!(
            r,
            Request::Sample {
                spp: 16,
                moving: 24,
                seed: 7
            }
        );
        let r: Request =
            serde_json::from_str(r#"{"op":"input","event":{"kind":"cue","cue":"deck:keep"}}"#)
                .unwrap();
        assert_eq!(
            r,
            Request::Input {
                event: Event::Cue {
                    cue: "deck:keep".into()
                }
            }
        );
        let r: Request = serde_json::from_str(
            r#"{"op":"frame","path":"f.rgba","ground":[1,1,1],"linear":true}"#,
        )
        .unwrap();
        assert_eq!(
            r,
            Request::Frame {
                path: "f.rgba".into(),
                ground: Some([1.0; 3]),
                linear: true
            }
        );
    }

    #[test]
    fn an_audio_request_names_a_path_and_a_length() {
        let r: Request =
            serde_json::from_str(r#"{"op":"audio","path":"a.wav","seconds":1.5}"#).unwrap();
        assert_eq!(
            r,
            Request::Audio {
                path: "a.wav".into(),
                seconds: 1.5
            }
        );
    }

    #[test]
    fn a_hello_without_audio_reads_as_silent() {
        let h: Hello = serde_json::from_str(
            r#"{"protocol":1,"app":"a","rev":"r","depth":16,"cutouts":false}"#,
        )
        .unwrap();
        assert!(!h.audio);
        assert_eq!(h.audio_format, None);
        let h: Hello = serde_json::from_str(
            r#"{"protocol":1,"app":"a","rev":"r","depth":16,"cutouts":false,"audio":true,"audio_format":"f32"}"#,
        )
        .unwrap();
        assert!(h.audio);
        assert_eq!(h.audio_format, Some(AudioFormat::F32));
    }

    #[test]
    fn replies_are_ok_or_err() {
        assert_eq!(
            serde_json::to_string(&Reply::Err("no".into())).unwrap(),
            r#"{"err":"no"}"#
        );
        assert_eq!(
            serde_json::to_string(&Reply::Ok(Value::Null)).unwrap(),
            r#"{"ok":null}"#
        );
    }

    #[test]
    fn an_adapter_that_names_no_backend_still_opens() {
        let o: Opened =
            serde_json::from_str(r#"{"size":[1280,720],"gpu":"g","driver":"d"}"#).unwrap();
        assert_eq!(o.backend, None);
    }
}
