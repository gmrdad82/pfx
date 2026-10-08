use std::path::Path;
use std::process::{Child, Command, Stdio};

pub const COLOR: &str = "BT.709 primaries, transfer and matrix, limited range (SDR)";

const TO_LIMITED: &str = "format=rgb48le,zscale=matrix=709:range=limited:filter=bicubic:chromal=left,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv";

const TO_ALPHA: &str = "zscale=matrix=709:range=limited,format=yuva444p10le,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv";

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

pub fn master_args(fps: u32) -> Vec<String> {
    let mut args = strings(&[
        "-vf",
        TO_LIMITED,
        "-c:v",
        "libx264",
        "-preset",
        "slow",
        "-profile:v",
        "high",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "12",
        "-g",
    ]);
    args.push(fps.to_string());
    args.push("-keyint_min".into());
    args.push(fps.to_string());
    args.extend(strings(&[
        "-sc_threshold",
        "0",
        "-threads",
        "16",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
        "-color_range",
        "tv",
        "-movflags",
        "+faststart+write_colr",
    ]));
    args
}

pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_BITRATE: &str = "320k";
pub const AUDIO_CODEC: &str = "AAC-LC";

pub fn audio_args(seconds: f64) -> Vec<String> {
    let mut args = vec![
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "1:a:0".into(),
        "-c:v".into(),
        "copy".into(),
        "-af".into(),
        format!("apad=whole_dur={seconds:.6},atrim=end={seconds:.6}"),
    ];
    args.extend(strings(&[
        "-c:a",
        "aac",
        "-profile:a",
        "aac_low",
        "-b:a",
        AUDIO_BITRATE,
        "-ar",
        "48000",
        "-ac",
        "2",
        "-fflags",
        "+bitexact",
        "-flags:a",
        "+bitexact",
        "-map_metadata",
        "-1",
        "-movflags",
        "+faststart+write_colr",
    ]));
    args
}

pub fn mux_audio(
    ffmpeg: &Path,
    video: &Path,
    pcm: &Path,
    seconds: f64,
    out: &Path,
) -> Result<(), String> {
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y", "-i"])
        .arg(video)
        .arg("-i")
        .arg(pcm)
        .args(audio_args(seconds))
        .arg(out)
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg could not mux the audio into {}",
            out.display()
        ))
    }
}

pub fn alpha_args() -> Vec<String> {
    strings(&[
        "-vf",
        TO_ALPHA,
        "-c:v",
        "prores_ks",
        "-profile:v",
        "4444",
        "-pix_fmt",
        "yuva444p10le",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
        "-color_range",
        "tv",
        "-movflags",
        "+write_colr",
    ])
}

pub fn raw(
    ffmpeg: &Path,
    width: u32,
    height: u32,
    fps: u32,
    args: &[String],
    out: &Path,
) -> Result<Child, String> {
    raw_command(ffmpeg, width, height, fps, args, out)
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))
}

fn raw_command(
    ffmpeg: &Path,
    width: u32,
    height: u32,
    fps: u32,
    args: &[String],
    out: &Path,
) -> Command {
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba64le",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "iec61966-2-1",
            "-colorspace",
            "rgb",
            "-color_range",
            "pc",
            "-s",
        ])
        .arg(format!("{width}x{height}"))
        .args(["-r", &fps.to_string(), "-i", "-", "-an"])
        .args(args)
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    command
}

pub fn file(ffmpeg: &Path, input: &Path, args: &[String], out: &Path) -> Result<(), String> {
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y", "-i"])
        .arg(input)
        .arg("-an")
        .args(args)
        .arg(out)
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("ffmpeg could not encode {}", out.display()))
    }
}

pub fn alpha_sequence(ffmpeg: &Path, cutouts: &Path, fps: u32, out: &Path) -> Result<(), String> {
    let input = cutouts.join("%05d.png");
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y", "-framerate"])
        .arg(fps.to_string())
        .arg("-i")
        .arg(input)
        .arg("-an")
        .args(alpha_args())
        .arg(out)
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("ffmpeg could not encode {}", out.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn x264_settings_match_the_master_convention() {
        let args = master_args(60).join(" ");
        assert!(args.contains("-c:v libx264 -preset slow -profile:v high -pix_fmt yuv420p -crf 12 -g 60 -keyint_min 60 -sc_threshold 0"));
        assert!(
            args.contains(
                "-color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv"
            )
        );
        assert!(args.ends_with("-movflags +faststart+write_colr"));
        assert!(args.contains("-threads 16 -filter_threads 1 -filter_complex_threads 1"));
        assert!(args.contains(
            "format=rgb48le,zscale=matrix=709:range=limited:filter=bicubic:chromal=left,format=yuv420p,"
        ));
    }

    #[test]
    fn audio_is_aac_lc_at_320k_and_48k_with_the_video_copied() {
        let args = audio_args(1.5).join(" ");
        assert!(args.contains("-c:v copy"));
        assert!(args.contains("-c:a aac -profile:a aac_low -b:a 320k -ar 48000 -ac 2"));
        assert!(args.contains("apad=whole_dur=1.500000,atrim=end=1.500000"));
        assert!(args.contains("-fflags +bitexact -flags:a +bitexact"));
        assert!(args.ends_with("-movflags +faststart+write_colr"));
    }

    #[test]
    fn prores_settings_preserve_alpha_and_tag_bt709() {
        let args = alpha_args().join(" ");
        assert!(args.contains("-c:v prores_ks -profile:v 4444 -pix_fmt yuva444p10le"));
        assert!(args.contains("zscale=matrix=709:range=limited,format=yuva444p10le,"));
        assert!(args.contains("-filter_threads 1 -filter_complex_threads 1"));
        assert!(
            args.contains(
                "-color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv"
            )
        );
    }

    #[test]
    fn alpha_master_keeps_its_colour_through_a_bt709_reader() {
        let pin = crate::ffmpeg::pin().unwrap();
        let ffmpeg = crate::ffmpeg::resolve(&crate::ffmpeg::cache().unwrap(), "pfx", &pin).unwrap();
        let ffprobe = ffmpeg.with_file_name("ffprobe");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("prores-colour-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("swatch.alpha.master.mov");
        let mut child = raw(&ffmpeg, 16, 16, 4, &alpha_args(), &output).unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let swatch = [192u16, 64, 32].map(|value| value * 257);
        for _ in 0..2 {
            for _ in 0..16 * 16 {
                for channel in [swatch[0], swatch[1], swatch[2], u16::MAX] {
                    stdin.write_all(&channel.to_le_bytes()).unwrap();
                }
            }
        }
        drop(stdin);
        assert!(child.wait().unwrap().success());
        let info = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_streams",
                "-of",
                "json",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert!(info.status.success());
        let json: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
        let stream = &json["streams"][0];
        assert_eq!(stream["codec_name"], "prores");
        assert_eq!(stream["color_primaries"], "bt709");
        assert_eq!(stream["color_transfer"], "bt709");
        assert_eq!(stream["color_space"], "bt709");
        assert_eq!(stream["color_range"], "tv");
        let stored = Command::new(&ffmpeg)
            .args(["-hide_banner", "-v", "error", "-i"])
            .arg(&output)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuva444p10le",
                "-",
            ])
            .output()
            .unwrap();
        assert!(stored.status.success());
        assert_eq!(
            yuva444p10_centre(&stored.stdout),
            [369, 404, 742, 1023],
            "stored 10-bit {}",
            stored.stdout.len()
        );
        let decoded = Command::new(&ffmpeg)
            .args(["-hide_banner", "-v", "error", "-i"])
            .arg(&output)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=in_color_matrix=bt709:in_range=tv",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-",
            ])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        let centre = &decoded.stdout[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4];
        for (got, want) in centre.iter().zip([192u8, 64, 32, 255]) {
            assert!(
                (i16::from(*got) - i16::from(want)).abs() <= 1,
                "decoded {centre:?} against (192, 64, 32, 255)"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checker_master_decodes_with_tags_and_one_second_keyframes() {
        let pin = crate::ffmpeg::pin().unwrap();
        let ffmpeg = crate::ffmpeg::resolve(&crate::ffmpeg::cache().unwrap(), "pfx", &pin).unwrap();
        let ffprobe = ffmpeg.with_file_name("ffprobe");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("x264-checker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("checker.master.mp4");
        let mut child = raw(&ffmpeg, 64, 64, 4, &master_args(4), &output).unwrap();
        let mut stdin = child.stdin.take().unwrap();
        for frame in 0..9 {
            for y in 0..64 {
                for x in 0..64 {
                    let light = (x / 8 + y / 8 + frame) % 2 == 0;
                    let value = if light { u16::MAX } else { 0u16 };
                    for channel in [value, value, value, u16::MAX] {
                        stdin.write_all(&channel.to_le_bytes()).unwrap();
                    }
                }
            }
        }
        drop(stdin);
        assert!(child.wait().unwrap().success());
        let info = Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_streams",
                "-show_frames",
                "-of",
                "json",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert!(info.status.success());
        let json: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
        let stream = &json["streams"][0];
        assert_eq!(stream["codec_name"], "h264");
        assert_eq!(stream["profile"], "High");
        assert_eq!(stream["pix_fmt"], "yuv420p");
        assert_eq!(stream["color_primaries"], "bt709");
        assert_eq!(stream["color_transfer"], "bt709");
        assert_eq!(stream["color_space"], "bt709");
        assert_eq!(stream["color_range"], "tv");
        let keys: Vec<usize> = json["frames"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(i, frame)| (frame["key_frame"] == 1).then_some(i))
            .collect();
        assert_eq!(keys, [0, 4, 8]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn x264_master_keeps_its_colour_through_a_bt709_reader() {
        let pin = crate::ffmpeg::pin().unwrap();
        let ffmpeg = crate::ffmpeg::resolve(&crate::ffmpeg::cache().unwrap(), "pfx", &pin).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("x264-swatch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("swatch.master.mp4");
        let mut child = raw(&ffmpeg, 16, 16, 4, &master_args(4), &output).unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let swatch = [192u16, 64, 32].map(|value| value * 257);
        for _ in 0..2 {
            for _ in 0..16 * 16 {
                for channel in [swatch[0], swatch[1], swatch[2], u16::MAX] {
                    stdin.write_all(&channel.to_le_bytes()).unwrap();
                }
            }
        }
        drop(stdin);
        assert!(child.wait().unwrap().success());
        let stored = Command::new(&ffmpeg)
            .args(["-hide_banner", "-v", "error", "-i"])
            .arg(&output)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuv420p",
                "-",
            ])
            .output()
            .unwrap();
        assert!(stored.status.success());
        assert_eq!(yuv420p_centre(&stored.stdout), [92, 101, 186]);
        let vf = master_args(4).into_iter().nth(1).unwrap();
        let ten = vf.replacen("format=yuv420p,", "format=yuv420p10le,", 1);
        let yuv = dir.join("swatch.10.yuv");
        let mut wide = raw(
            &ffmpeg,
            16,
            16,
            4,
            &[
                "-vf".into(),
                ten,
                "-frames:v".into(),
                "1".into(),
                "-f".into(),
                "rawvideo".into(),
                "-pix_fmt".into(),
                "yuv420p10le".into(),
            ],
            &yuv,
        )
        .unwrap();
        let mut stdin = wide.stdin.take().unwrap();
        let swatch = [192u16, 64, 32].map(|value| value * 257);
        for _ in 0..16 * 16 {
            for channel in [swatch[0], swatch[1], swatch[2], u16::MAX] {
                stdin.write_all(&channel.to_le_bytes()).unwrap();
            }
        }
        drop(stdin);
        assert!(wide.wait().unwrap().success());
        assert_eq!(
            yuv420p10_centre(&std::fs::read(yuv).unwrap()),
            [369, 404, 742]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    fn frames_rgba64(width: usize, height: usize, count: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut out = Vec::with_capacity(width * height * count * 8);
        for frame in 0..count {
            for y in 0..height {
                for x in 0..width {
                    state = state
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let noise = (state >> 48) as u16 & 0x03ff;
                    let r = ((x * 65535 / width) as u16).wrapping_add(noise);
                    let g = ((y * 65535 / height) as u16).wrapping_add((frame * 2111) as u16);
                    let b = (((x + y + frame * 5) * 997) as u16) ^ noise;
                    let a = ((x * 65535 / width) as u16) | 0x4000;
                    for channel in [r, g, b, a] {
                        out.extend_from_slice(&channel.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    #[cfg(target_os = "linux")]
    fn encode_on(
        cpus: Option<&str>,
        ffmpeg: &Path,
        args: &[String],
        frames: &[u8],
        out: &Path,
    ) -> Vec<u8> {
        let mut command = raw_command(ffmpeg, 320, 180, 6, args, out);
        if let Some(cpus) = cpus {
            let mut pinned = Command::new("taskset");
            pinned
                .arg("-c")
                .arg(cpus)
                .arg(command.get_program())
                .args(command.get_args())
                .stdin(Stdio::piped())
                .stdout(Stdio::null());
            command = pinned;
        }
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(frames).unwrap();
        drop(stdin);
        assert!(child.wait().unwrap().success());
        std::fs::read(out).unwrap()
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn masters_are_byte_identical_on_any_cpu_count() {
        let pin = crate::ffmpeg::pin().unwrap();
        let ffmpeg = crate::ffmpeg::resolve(&crate::ffmpeg::cache().unwrap(), "pfx", &pin).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("master-threads-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frames = frames_rgba64(320, 180, 12);
        for (name, args) in [
            ("a.master.mp4", master_args(6)),
            ("a.alpha.master.mov", alpha_args()),
        ] {
            let one = encode_on(Some("0"), &ffmpeg, &args, &frames, &dir.join(name));
            let all = encode_on(None, &ffmpeg, &args, &frames, &dir.join(name));
            assert!(!one.is_empty());
            assert!(one == all, "{name} differs between 1 CPU and all CPUs");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn sample16(buf: &[u8], start: usize, stride: usize, x: usize, y: usize) -> u16 {
        let i = (start + y * stride + x) * 2;
        u16::from_le_bytes([buf[i], buf[i + 1]])
    }

    fn yuva444p10_centre(buf: &[u8]) -> [u16; 4] {
        let n = 16 * 16;
        [0, 1, 2, 3].map(|plane| sample16(buf, plane * n, 16, 8, 8))
    }

    fn yuv420p_centre(buf: &[u8]) -> [u8; 3] {
        let n = 16 * 16;
        let cn = 8 * 8;
        [buf[8 * 16 + 8], buf[n + 4 * 8 + 4], buf[n + cn + 4 * 8 + 4]]
    }

    fn yuv420p10_centre(buf: &[u8]) -> [u16; 3] {
        let n = 16 * 16;
        let cn = 8 * 8;
        [
            sample16(buf, 0, 16, 8, 8),
            sample16(buf, n, 8, 4, 4),
            sample16(buf, n + cn, 8, 4, 4),
        ]
    }
}
