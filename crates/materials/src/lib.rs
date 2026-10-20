mod ageing;
mod brdf;
mod colour;
mod content;
mod detail;
mod film;
pub mod fixtures;
mod flakes;
mod kinds;
mod library;
mod lobes;
mod material;
mod math;
mod noise;
mod pavement;
mod wet;
mod wgsl;

pub use ageing::{
    apply, dust, ink_age, patina, scratch, scratch_coat, scratch_marked, scratch_marked_coat,
    worn_edge, yellow,
};
pub use brdf::{
    GLASS_DISPERSION_BLUE, GLASS_DISPERSION_RED, GLASS_IOR, PI, RGB_WAVELENGTHS_NM, Shaded,
    WATER_IOR, alpha, beer, dielectric_f0, dispersed_ior, eval, film_rgb, fresnel_conductor,
    fresnel_dielectric, ggx_d, ior_from_f0, lambert, liquid_dye, metal_film, refract, sample_vndf,
    schlick, smith_correlated, smith_g, smith_g1, thin_dielectric,
};
pub use colour::{encode_channel, linear_channel, srgb_hex};
pub use content::{Blend, ContentLayer, ContentLook, content_uv, emboss};
pub use detail::{Resolved, resolve};
pub use film::{
    FILM_EDGE, FILM_NU, FILM_SAMPLES, FILM_STEP, FILM_WEIGHTS, conductor_ior, film_conductor,
    film_dielectric, film_wavelengths,
};
pub use flakes::{
    Cell, FLAKE_RANDOMNESS, Flakes, flake_frame, flake_normal, hash_int3_to_float3, hash_pcg3d_i,
    voronoi_f1,
};
pub use kinds::{
    CoatWobble, Crack, Crinkle, Fibre, FilmRamp, Grime, PlankWood, RampSpace, Scratch, Track,
    WallMottle, sample_params,
};
pub use library::{FIXTURE, Library};
pub use lobes::{
    Anisotropy, Fabric, SHEEN_ROUGHNESS_FLOOR, SHEEN_TABLE, SHEEN_TABLE_SIDE, Scatter,
    THIN_FILM_MIN, Textures, UvTransform, d_charlie, sheen_albedo, sheen_albedo_exact, sheen_alpha,
    v_neubelt,
};
pub use material::{
    ALPHA_CUTOFF, Ageing, AlphaMode, At, Content, Family, GPU_PARAMS, LAYER_CAP, LOBE_ROWS, Maps,
    Material, NoiseKind, NoiseLayer, Normal, NormalSource, PARAM_ROWS, PARAMS, PackedMaterial,
    SCRATCH_STRETCH, gpu_params,
};
pub use noise::{
    Hash, fbm3, fbm3_filtered, flow, hash2, hash3, hash21, noise2, octave_weight, pcg,
    value_noise2_signed, value_noise3, value_noise3_gradient,
};
pub use pavement::{crack, plane, track, voronoi_edge2};
pub use wet::{
    PORE_SPAN, PORE_START, PUDDLE_EDGE, PUDDLE_FLAT, PUDDLE_MEDIAN, PUDDLE_OCTAVES,
    PUDDLE_SCALE_MIN, PUDDLE_SPREAD, Soaked, WATER_ROUGHNESS, WET_DARKEST, Wet, puddle, soak,
};
pub use wgsl::{AGEING, BRDF, CONTENT, FILM, LOBES, NOISE, WET, library};

#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wet_tests;
