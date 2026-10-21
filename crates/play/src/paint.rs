use pfx_gpu::wgpu;
use pfx_live::flat::look::Look;
use pfx_live::renderer::Renderer;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintFrame {
    pub format: wgpu::TextureFormat,
    pub output: [u32; 2],
    pub content: [f32; 4],
    pub clip: Option<[u32; 4]>,
    pub render: Option<[u32; 2]>,
    pub layout: [f32; 2],
    pub content_units: [f32; 4],
    pub window_units: [f32; 2],
    pub pixels_per_unit: f32,
    pub frame: u64,
    pub seed: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Warming {
    #[default]
    Done,
    More,
}

pub trait Warmer {
    fn glyphs(
        &mut self,
        family: &str,
        weight: u16,
        sizes: &[f32],
        chars: &str,
    ) -> Result<usize, String>;

    fn looks(&mut self, looks: &[Look]) -> Result<(), String>;

    fn renderer(&mut self) -> Option<&mut Renderer>;

    fn format(&self) -> Option<wgpu::TextureFormat> {
        None
    }

    fn size(&self) -> Option<[u32; 2]> {
        None
    }
}

pub struct Warm<'a> {
    warmer: &'a mut dyn Warmer,
    pass: u32,
    elapsed: f64,
    budget: f64,
}

impl<'a> Warm<'a> {
    pub fn new(warmer: &'a mut dyn Warmer, pass: u32, elapsed: f64, budget: f64) -> Self {
        Self {
            warmer,
            pass,
            elapsed,
            budget,
        }
    }

    pub fn glyphs(
        &mut self,
        family: &str,
        weight: u16,
        sizes: &[f32],
        chars: &str,
    ) -> Result<usize, String> {
        self.warmer.glyphs(family, weight, sizes, chars)
    }

    pub fn looks(&mut self, looks: &[Look]) -> Result<(), String> {
        self.warmer.looks(looks)
    }

    pub fn renderer(&mut self) -> Option<&mut Renderer> {
        self.warmer.renderer()
    }

    pub fn format(&self) -> Option<wgpu::TextureFormat> {
        self.warmer.format()
    }

    pub fn size(&self) -> Option<[u32; 2]> {
        self.warmer.size()
    }

    pub fn pass(&self) -> u32 {
        self.pass
    }

    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    pub fn budget(&self) -> f64 {
        self.budget
    }

    pub fn left(&self) -> f64 {
        (self.budget - self.elapsed).max(0.0)
    }
}

pub struct NoWarmer;

impl Warmer for NoWarmer {
    fn glyphs(
        &mut self,
        _family: &str,
        _weight: u16,
        _sizes: &[f32],
        _chars: &str,
    ) -> Result<usize, String> {
        Ok(0)
    }

    fn looks(&mut self, _looks: &[Look]) -> Result<(), String> {
        Ok(())
    }

    fn renderer(&mut self) -> Option<&mut Renderer> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sized(Option<[u32; 2]>);

    impl Warmer for Sized {
        fn glyphs(
            &mut self,
            _family: &str,
            _weight: u16,
            _sizes: &[f32],
            _chars: &str,
        ) -> Result<usize, String> {
            Ok(0)
        }

        fn looks(&mut self, _looks: &[Look]) -> Result<(), String> {
            Ok(())
        }

        fn renderer(&mut self) -> Option<&mut Renderer> {
            None
        }

        fn size(&self) -> Option<[u32; 2]> {
            self.0
        }
    }

    #[test]
    fn warm_hands_the_game_its_warmers_size_and_none_without_one() {
        let mut sized = Sized(Some([2560, 1440]));
        let warm = Warm::new(&mut sized, 0, 0.0, 2.0);
        assert_eq!(warm.size(), Some([2560, 1440]));
        assert_eq!(warm.format(), None);
        let mut bare = Sized(None);
        assert_eq!(Warm::new(&mut bare, 1, 0.5, 2.0).size(), None);
        let mut none = NoWarmer;
        let warm = Warm::new(&mut none, 0, 0.0, 2.0);
        assert_eq!(warm.size(), None);
        assert_eq!(warm.format(), None);
    }
}
