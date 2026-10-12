@group(0) @binding(19) var sobol_table: texture_2d<f32>;
fn reversed_bit_owen(n0: u32, seed: u32) -> u32 {
    var n=n0;
    n^=n*0x3d20adeau;
    n+=seed;
    n*=(seed>>16u)|1u;
    n^=n*0x05526c56u;
    n^=n*0x53a22864u;
    return n;
}
fn nested_uniform_scramble(i: u32, seed: u32) -> u32 {
    return reverseBits(reversed_bit_owen(reverseBits(i),seed));
}
fn hash_wang_seeded_uint(i0: u32, seed: u32) -> u32 {
    var i=(i0^61u)^seed;
    i+=i<<3u;
    i^=i>>4u;
    i*=0x27d4eb2du;
    return i;
}
fn hash_shuffle_uint(i0: u32, length: u32, seed: u32) -> u32 {
    var i=i0%length;
    let mask=(1u<<(32u-countLeadingZeros(length-1u)))-1u;
    loop {
        i^=seed;
        i*=0xe170893du;
        i^=seed>>16u;
        i^=(i&mask)>>4u;
        i^=seed>>8u;
        i*=0x0929eb3fu;
        i^=seed>>23u;
        i^=(i&mask)>>1u;
        i*=1u|(seed>>27u);
        i*=0x6935fa69u;
        i^=(i&mask)>>11u;
        i*=0x74dcb303u;
        i^=(i&mask)>>2u;
        i*=0x9e501cc3u;
        i^=(i&mask)>>2u;
        i*=0xc860a3dfu;
        i&=mask;
        i^=i>>5u;
        if (i<length) { break; }
    }
    return i;
}
fn sobol(dimension: u32) -> vec4f {
    if (!SOBOL) { return vec4f(0.0); }
    let seed=rng.x;
    let dim=rng.z+dimension;
    let pattern=hash_shuffle_uint(dim,256u,seed);
    let mask=SOBOL_LENGTH-1u;
    let shuffled=nested_uniform_scramble(rng.y,hash_wang_seeded_uint(dim,seed));
    let sample=(rng.y&~mask)|(shuffled&mask);
    let index=(pattern*SOBOL_LENGTH+sample)%(SOBOL_LENGTH*256u);
    return min(textureLoad(sobol_table,vec2u(index&1023u,index>>10u),0),vec4f(0.99999994));
}
