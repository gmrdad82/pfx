use pfx_post::{Bloom, Chain, Pass, Tone};
use pfx_scene::types as file;
use sha2::{Digest, Sha256};

use super::Finish;

pub(super) fn flare(found: &file::Flare) -> pfx_post::Flare {
    let base = pfx_post::Flare::default();
    pfx_post::Flare {
        amount: found.amount.unwrap_or(base.amount),
        ghosts: found.ghosts.unwrap_or(base.ghosts),
        ghost_spacing: found.ghost_spacing.unwrap_or(base.ghost_spacing),
        halo: found.halo.unwrap_or(base.halo),
        halo_radius: found.halo_radius.unwrap_or(base.halo_radius),
        starburst: found.starburst.unwrap_or(base.starburst),
        threshold: found.threshold.unwrap_or(base.threshold),
    }
}

fn standard() -> Chain {
    Chain {
        passes: vec![Pass::Bloom(Bloom::neutral(0.0)), Pass::Tone(Tone::aces())],
        seed: 0,
        frame: 0,
    }
}

pub(super) fn fold(finish: Option<Finish>, found: Option<&file::Flare>) -> Option<Finish> {
    let Some(found) = found else {
        return finish;
    };
    let pass = flare(found);
    let (mut chain, hash) = match finish {
        Some(finish) => (finish.chain, finish.hash),
        None => (standard(), [0; 32]),
    };
    chain.set_flare(pass);
    let mut hasher = Sha256::new();
    hasher.update(hash);
    hasher.update(b"flare");
    for value in [
        pass.amount,
        pass.ghost_spacing,
        pass.halo,
        pass.halo_radius,
        pass.starburst,
        pass.threshold,
    ] {
        hasher.update(value.to_le_bytes());
    }
    hasher.update(pass.ghosts.to_le_bytes());
    Some(Finish {
        chain,
        hash: hasher.finalize().into(),
    })
}
