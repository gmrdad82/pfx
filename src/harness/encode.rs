pub fn stem(product: &str, shot: &str, width: u32, height: u32, fps: u32) -> String {
    format!("{product}-clip-{shot}-{width}x{height}-{fps}fps")
}

pub fn poster(stem: &str) -> String {
    format!("{stem}.poster.png")
}

pub fn master(stem: &str) -> String {
    format!("{stem}.master.mp4")
}
