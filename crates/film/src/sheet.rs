use crate::script::Drawn;
use pfx_live::flat::{Draw, Fill, Material, Matrix, Shadow, Srgba, Stroke};

#[derive(Clone, Debug, PartialEq)]
pub struct Letter {
    pub text: String,
    pub size: f32,
    pub weight: u16,
    pub colour: Srgba,
    pub at: [f32; 2],
    pub frame: Matrix,
    pub elevation: f32,
    pub opacity: f32,
    pub group: Option<u16>,
}

pub trait Sheet {
    type Drawn: Drawn;
    fn size(&self) -> [f32; 2];
    fn draws(&mut self) -> &mut Vec<Draw>;
    fn group(&mut self, opacity: f32) -> Option<u16>;
    fn letter(&mut self, letter: Letter);
    fn width(&mut self, text: &str, size: f32, weight: u16) -> f32;
    fn check(&self, centre: [f32; 2], height: f32) -> Option<Draw>;
    fn art(&mut self, art: &Self::Drawn);

    fn add(&mut self, draw: Draw) {
        self.draws().push(draw);
    }
}

pub fn painted(mut draw: Draw, fill: Option<Srgba>, stroke: Option<(Srgba, f32)>) -> Draw {
    draw.fill = fill.map_or(Fill::None, Fill::Solid);
    draw.stroke = stroke.map(|(c, w)| Stroke::solid(c, w));
    draw.elevation = 0.0;
    draw.opacity = 1.0;
    draw.shadow = Shadow::None;
    draw.material = Material::default();
    draw.group = None;
    draw
}
