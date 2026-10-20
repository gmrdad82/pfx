#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aux {
    #[default]
    Noisy,
    Prefiltered,
    Clean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    Albedo,
    Normal,
    Color,
}

const CLEAN: &[u8] = include_bytes!("../weights/rt_hdr_calb_cnrm.tza");
const ALBEDO: &[u8] = include_bytes!("../weights/rt_alb.tza");
const NORMAL: &[u8] = include_bytes!("../weights/rt_nrm.tza");

impl Aux {
    pub(crate) fn filters(self) -> &'static [Filter] {
        match self {
            Aux::Prefiltered => &[Filter::Albedo, Filter::Normal, Filter::Color],
            Aux::Noisy | Aux::Clean => &[Filter::Color],
        }
    }
}

impl Filter {
    pub(crate) fn weights(self, aux: Aux) -> &'static [u8] {
        match (self, aux) {
            (Filter::Albedo, _) => ALBEDO,
            (Filter::Normal, _) => NORMAL,
            (Filter::Color, Aux::Noisy) => crate::WEIGHTS,
            (Filter::Color, Aux::Prefiltered | Aux::Clean) => CLEAN,
        }
    }

    pub(crate) fn input_channels(self) -> u32 {
        match self {
            Filter::Albedo | Filter::Normal => 3,
            Filter::Color => 9,
        }
    }

    pub(crate) fn entries(self) -> [&'static str; 2] {
        match self {
            Filter::Albedo => ["input_albedo", "output_albedo"],
            Filter::Normal => ["input_normal", "output_normal"],
            Filter::Color => ["input", "output"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_builds_its_networks_from_the_shipped_weights() {
        for aux in [Aux::Noisy, Aux::Prefiltered, Aux::Clean] {
            for &filter in aux.filters() {
                let tensors = crate::tza::parse(filter.weights(aux)).unwrap();
                let net = crate::net::build(&tensors, filter.input_channels()).unwrap();
                assert_eq!(net.layers.len(), 16, "{aux:?} {filter:?}");
                assert_eq!(net.layers[0].in0, filter.input_channels());
            }
        }
        assert_eq!(Aux::default(), Aux::Noisy);
        assert_eq!(Aux::Prefiltered.filters().last(), Some(&Filter::Color));
    }
}
