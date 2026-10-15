use crate::grade::{self, Grade};
use crate::marks::Marks;
use crate::pieces::{self, Kit, Piece};
use crate::sheet::Sheet;
use crate::titles::Setter;
use pfx_live::flat::{Draw, Fill, Gradient, Shadow, Srgba};

const STOPS: usize = 6;

pub struct Overlay {
    pub marks: Marks,
    setter: Setter,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn vignette<S: Sheet>(b: &mut S, strength: f32) {
    let size = b.size();
    let reach = grade::VIGNETTE[1];
    let inner = grade::VIGNETTE[0] / grade::VIGNETTE[1];
    let mut stops = vec![(0.0, [0.0; 4]), (inner, [0.0; 4])];
    for i in 1..=STOPS {
        let t = inner + (1.0 - inner) * i as f32 / STOPS as f32;
        let a = strength * smooth((t - inner) / (1.0 - inner));
        stops.push((t, [0.0, 0.0, 0.0, a]));
    }
    let Ok(gradient) = Gradient::radial([0.0, 0.0], [reach, reach], &stops) else {
        return;
    };
    let mut draw = Draw::rect([size[0] * 0.5, size[1] * 0.5], size, 0.0).shadow(Shadow::None);
    draw.fill = Fill::Gradient(gradient);
    b.add(draw);
}

fn under<S: Sheet>(b: &mut S, grade: &Grade, graded: bool) {
    if !grade.game || graded {
        return;
    }
    if grade.dim > 0.0 {
        let a = grade.dim.clamp(0.0, 1.0);
        let size = b.size();
        b.add(
            Draw::rect([size[0] * 0.5, size[1] * 0.5], size, 0.0)
                .fill(Srgba([0.0, 0.0, 0.0, a]))
                .shadow(Shadow::None),
        );
    }
    if grade.vignette > 0.0 {
        vignette(b, grade.vignette.min(1.0));
    }
}

impl Overlay {
    pub fn new(marks: Marks) -> Overlay {
        Overlay {
            marks,
            setter: Setter::default(),
        }
    }

    pub fn paint<S: Sheet>(
        &mut self,
        b: &mut S,
        pieces: &[Piece<S::Drawn>],
        grade: &Grade,
        graded: bool,
    ) {
        under(b, grade, graded);
        let mut kit = Kit {
            marks: &self.marks,
            setter: &mut self.setter,
            glow: grade.glow > 0.0,
        };
        for piece in pieces {
            pieces::place(b, &mut kit, piece);
        }
    }
}
