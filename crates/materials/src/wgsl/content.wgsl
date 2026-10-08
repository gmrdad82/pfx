fn content_composite(surface: Shaded, blend: u32, texel: vec4f, ink_roughness: f32, strength: f32) -> Shaded {
    var out = surface;
    let a = clamp(texel.a, 0.0, 1.0);
    if (blend == 0u) {
        out.base = surface.base * (1.0 - a) + texel.rgb;
        out.emission = surface.emission * (1.0 - a);
        if (ink_roughness >= 0.0) {
            let alpha = surface.roughness * surface.roughness;
            out.roughness = sqrt(mix(alpha, ink_roughness * ink_roughness, a));
        }
        out.subsurface = surface.subsurface * pow(1.0 - a, 4.0);
    } else if (blend == 1u) {
        out.base = surface.base * (vec3f(1.0 - a) + texel.rgb * strength);
    } else if (blend == 2u) {
        out.emission = surface.emission + texel.rgb * strength;
    }
    return out;
}

fn content_uv(uv: vec2f, transform: vec4f) -> vec2f {
    return uv * transform.zw + transform.xy;
}

fn content_inside(at: vec2f, crop: vec4f) -> bool {
    return all(at >= crop.xy) && all(at <= crop.zw);
}

fn content_emboss(n: vec3f, t: vec3f, b: vec3f, gradient: vec2f, strength: f32) -> vec3f {
    let tilt = (t * gradient.x + b * gradient.y) * strength;
    return normalize(n - (tilt - dot(tilt, n) * n));
}
