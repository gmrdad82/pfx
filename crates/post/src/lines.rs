#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lines {
    pub enabled: bool,
    pub thickness: f32,
    pub color: [f32; 3],
    pub alpha: f32,
    pub crease_angle: f32,
    pub silhouette: bool,
    pub border: bool,
    pub crease: bool,
    pub contour: bool,
    pub external_contour: bool,
}

impl Default for Lines {
    fn default() -> Self {
        Self {
            enabled: false,
            thickness: 2.0,
            color: [
                crate::view::srgb_decode(0x17 as f32 / 255.0),
                crate::view::srgb_decode(0x1a as f32 / 255.0),
                crate::view::srgb_decode(0x34 as f32 / 255.0),
            ],
            alpha: 1.0,
            crease_angle: 134.0,
            silhouette: true,
            border: true,
            crease: true,
            contour: false,
            external_contour: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    External,
    Silhouette,
    Contour,
    Border,
    Crease,
}

pub struct Gbuffer<'a> {
    pub width: u32,
    pub height: u32,
    pub depth: &'a [f32],
    pub id: &'a [u32],
    pub normal: &'a [[f32; 3]],
}

impl Gbuffer<'_> {
    fn valid(&self) -> bool {
        let n = self.width as usize * self.height as usize;
        self.depth.len() == n && self.id.len() == n && self.normal.len() == n
    }

    fn at(&self, x: i64, y: i64) -> Option<usize> {
        (x >= 0 && y >= 0 && x < i64::from(self.width) && y < i64::from(self.height))
            .then(|| y as usize * self.width as usize + x as usize)
    }

    pub fn classify(
        &self,
        a: usize,
        b: usize,
        before: Option<usize>,
        after: Option<usize>,
        crease_angle: f32,
    ) -> Option<Edge> {
        let (ia, ib) = (self.id[a], self.id[b]);
        match (ia != 0, ib != 0) {
            (false, false) => return None,
            (true, false) | (false, true) => return Some(Edge::External),
            _ => {}
        }
        let (za, zb) = (self.depth[a], self.depth[b]);
        let slope = |p: Option<usize>, q: usize| {
            p.filter(|&p| self.id[p] == self.id[q])
                .map_or(0.0, |p| (self.depth[p] - self.depth[q]).abs())
        };
        let local = slope(before, a).max(slope(after, b));
        let jump = (zb - za).abs();
        if jump > 2.0 * local + 0.002 * za.max(zb) {
            return Some(if ia != ib {
                Edge::Silhouette
            } else {
                Edge::Contour
            });
        }
        if ia != ib {
            return Some(Edge::Border);
        }
        let (na, nb) = (self.normal[a], self.normal[b]);
        let cosine = na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2];
        let limit = (180.0 - crease_angle).to_radians().cos();
        (cosine < limit).then_some(Edge::Crease)
    }
}

impl Lines {
    pub fn width_px(&self, image_width: u32) -> f32 {
        self.thickness * image_width as f32 / 1024.0
    }

    pub fn draws(&self, edge: Edge) -> bool {
        match edge {
            Edge::External => self.external_contour || self.silhouette,
            Edge::Silhouette => self.silhouette,
            Edge::Contour => self.contour,
            Edge::Border => self.border || self.crease,
            Edge::Crease => self.crease,
        }
    }

    pub fn segments(&self, g: &Gbuffer) -> Result<Vec<[f32; 4]>, String> {
        if !g.valid() {
            return Err("the outline's depth, id and normal do not match its size".into());
        }
        let mut out = Vec::new();
        for y in 0..i64::from(g.height) {
            for x in 0..i64::from(g.width) {
                let a = g.at(x, y).unwrap();
                if let Some(b) = g.at(x + 1, y)
                    && let Some(edge) =
                        g.classify(a, b, g.at(x - 1, y), g.at(x + 2, y), self.crease_angle)
                    && self.draws(edge)
                {
                    let edge_x = x as f32 + 1.0;
                    out.push([edge_x, y as f32, edge_x, y as f32 + 1.0]);
                }
                if let Some(b) = g.at(x, y + 1)
                    && let Some(edge) =
                        g.classify(a, b, g.at(x, y - 1), g.at(x, y + 2), self.crease_angle)
                    && self.draws(edge)
                {
                    let edge_y = y as f32 + 1.0;
                    out.push([x as f32, edge_y, x as f32 + 1.0, edge_y]);
                }
            }
        }
        Ok(out)
    }

    pub fn coverage(&self, g: &Gbuffer, image_width: u32) -> Result<Vec<f32>, String> {
        let segments = self.segments(g)?;
        let half = self.width_px(image_width) * 0.5;
        let reach = (half + 1.0).ceil() as i64;
        let (w, h) = (i64::from(g.width), i64::from(g.height));
        let mut coverage = vec![0.0f32; (w * h) as usize];
        for [x0, y0, x1, y1] in segments {
            let (cx, cy) = (
                ((x0 + x1) * 0.5).floor() as i64,
                ((y0 + y1) * 0.5).floor() as i64,
            );
            for py in (cy - reach).max(0)..(cy + reach + 1).min(h) {
                for px in (cx - reach).max(0)..(cx + reach + 1).min(w) {
                    let p = [px as f32 + 0.5, py as f32 + 0.5];
                    let d = segment_distance(p, [x0, y0], [x1, y1]);
                    let k = (half + 0.5 - d).clamp(0.0, 1.0);
                    let cell = &mut coverage[(py * w + px) as usize];
                    *cell = cell.max(k);
                }
            }
        }
        Ok(coverage)
    }

    pub fn draw(&self, premultiplied: &mut [[f32; 4]], coverage: &[f32]) -> Result<(), String> {
        if premultiplied.len() != coverage.len() {
            return Err("the outline's coverage does not match the image".into());
        }
        for (texel, &cover) in premultiplied.iter_mut().zip(coverage) {
            let k = (cover * self.alpha).clamp(0.0, 1.0);
            for (value, colour) in texel.iter_mut().zip(self.color) {
                *value = colour * k + *value * (1.0 - k);
            }
            texel[3] = k + texel[3] * (1.0 - k);
        }
        Ok(())
    }
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-12))
        .clamp(0.0, 1.0);
    let q = [a[0] + ab[0] * t - p[0], a[1] + ab[1] * t - p[1]];
    (q[0] * q[0] + q[1] * q[1]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        width: u32,
        height: u32,
        depth: Vec<f32>,
        id: Vec<u32>,
        normal: Vec<[f32; 3]>,
    }

    impl Fixture {
        fn new(width: u32, height: u32) -> Self {
            let n = (width * height) as usize;
            Self {
                width,
                height,
                depth: vec![0.0; n],
                id: vec![0; n],
                normal: vec![[0.0; 3]; n],
            }
        }

        fn fill(
            &mut self,
            x: std::ops::Range<u32>,
            y: std::ops::Range<u32>,
            depth: f32,
            id: u32,
            normal: [f32; 3],
        ) {
            for yy in y {
                for xx in x.clone() {
                    let i = (yy * self.width + xx) as usize;
                    self.depth[i] = depth;
                    self.id[i] = id;
                    self.normal[i] = normal;
                }
            }
        }

        fn g(&self) -> Gbuffer<'_> {
            Gbuffer {
                width: self.width,
                height: self.height,
                depth: &self.depth,
                id: &self.id,
                normal: &self.normal,
            }
        }
    }

    #[test]
    fn a_square_gets_its_external_contour() {
        let mut f = Fixture::new(32, 32);
        f.fill(8..24, 8..24, 5.0, 1, [0.0, 0.0, 1.0]);
        let lines = Lines {
            enabled: true,
            ..Lines::default()
        };
        let cover = lines.coverage(&f.g(), 1024).unwrap();
        let at = |x: usize, y: usize| cover[y * 32 + x];
        assert!((at(7, 16) - 1.0).abs() < 1e-6 && (at(8, 16) - 1.0).abs() < 1e-6);
        assert_eq!((at(6, 16), at(9, 16)), (0.0, 0.0));
        assert_eq!(at(16, 16), 0.0);
        assert_eq!(at(2, 2), 0.0);
        let thin = lines.coverage(&f.g(), 512).unwrap();
        assert!((thin[16 * 32 + 7] - 0.5).abs() < 1e-6 && (thin[16 * 32 + 8] - 0.5).abs() < 1e-6);
        let off = Lines {
            external_contour: false,
            silhouette: false,
            ..lines
        };
        assert!(off.segments(&f.g()).unwrap().is_empty());
    }

    #[test]
    fn creases_follow_the_crease_angle() {
        let mut f = Fixture::new(16, 8);
        f.fill(0..8, 0..8, 4.0, 1, [0.0, 0.0, 1.0]);
        let tilt = 50.0f32.to_radians();
        f.fill(8..16, 0..8, 4.0, 1, [tilt.sin(), 0.0, tilt.cos()]);
        let g = f.g();
        assert_eq!(
            g.classify(7, 8, Some(6), Some(9), 134.0),
            Some(Edge::Crease)
        );
        assert_eq!(g.classify(7, 8, Some(6), Some(9), 120.0), None);
        let lines = Lines::default();
        let segments = lines.segments(&g).unwrap();
        assert_eq!(segments.len(), 8);
        assert!(segments.iter().all(|s| s[0] == 8.0 && s[2] == 8.0));
        assert!(
            Lines {
                crease: false,
                ..lines
            }
            .segments(&g)
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn depth_jumps_are_silhouettes_or_contours_and_slopes_are_not() {
        let mut f = Fixture::new(12, 4);
        for x in 0..12 {
            f.fill(x..x + 1, 0..4, 2.0 + 0.3 * x as f32, 1, [0.0, 0.0, 1.0]);
        }
        let g = f.g();
        assert_eq!(g.classify(4, 5, Some(3), Some(6), 134.0), None);
        f.fill(6..12, 0..4, 9.0, 1, [0.0, 0.0, 1.0]);
        assert_eq!(
            f.g().classify(5, 6, Some(4), Some(7), 134.0),
            Some(Edge::Contour)
        );
        f.fill(6..12, 0..4, 9.0, 2, [0.0, 0.0, 1.0]);
        assert_eq!(
            f.g().classify(5, 6, Some(4), Some(7), 134.0),
            Some(Edge::Silhouette)
        );
        f.fill(6..12, 0..4, 2.0 + 0.3 * 6.0, 2, [0.0, 0.0, 1.0]);
        assert_eq!(
            f.g().classify(5, 6, Some(4), Some(7), 134.0),
            Some(Edge::Border)
        );
        assert!(!Lines::default().draws(Edge::Contour));
    }

    #[test]
    fn lines_draw_over_premultiplied_alpha() {
        let lines = Lines {
            color: [1.0, 0.0, 0.0],
            alpha: 0.5,
            ..Lines::default()
        };
        let mut image = vec![[0.0, 0.0, 0.4, 0.5], [0.0; 4]];
        lines.draw(&mut image, &[1.0, 0.0]).unwrap();
        assert_eq!(image[0], [0.5, 0.0, 0.2, 0.75]);
        assert_eq!(image[1], [0.0; 4]);
        assert!(lines.draw(&mut image, &[1.0]).is_err());
        assert!((lines.width_px(2048) - 4.0).abs() < 1e-6);
    }
}
