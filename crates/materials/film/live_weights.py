import os
import sys

import numpy as np

sys.dont_write_bytecode = True

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "trace", "film"))
from film_weights import literal, weights

SAMPLES = 24
SHORT = 390.0
LONG = 740.0
TRACER_WAVES = 380.0 + 5.0 * np.arange(81)
TRACER = weights(os.path.join(HERE, "..", "..", "trace", "film", "wavelength.csv"))
DIELECTRICS = [(1.9, 1.4), (1.33, 1.5), (1.5, 2.4), (2.0, 1.5), (2.6, 1.5), (1.45, 1.0), (2.3, 1.6), (1.6, 1.33)]
METALS = [(0.95, 0.64, 0.54), (1.0, 0.78, 0.34), (0.56, 0.57, 0.58), (0.91, 0.92, 0.92), (0.2, 0.3, 0.8)]
METAL_FILMS = [1.5, 2.0, 2.6]
LEAST_COSINE = 0.2
DIELECTRIC_REACH = 1100.0
METAL_REACH = 400.0
RIDGE = 30.0


def waves(samples=SAMPLES):
    return np.linspace(1.0 / LONG, 1.0 / SHORT, samples)


def amplitudes(cos_i, na, nb):
    sin2 = (na / nb) ** 2 * (1.0 - cos_i * cos_i)
    cos_t = np.sqrt(np.maximum(1.0 - sin2, 0.0))
    rs = (na * cos_i - nb * cos_t) / (na * cos_i + nb * cos_t)
    rp = (nb * cos_i - na * cos_t) / (nb * cos_i + na * cos_t)
    return rs, rp, cos_t


def conductor_ior(f0):
    r = min(f0, 0.999)
    edge = f0 + (1.0 - f0) * (6.0 / 7.0) ** 5
    root = np.sqrt(r)
    n = (1.0 + root) / (1.0 - root) * (1.0 - edge) + (1.0 - r) / (1.0 + r) * edge
    k = np.sqrt(max((r * (n + 1.0) ** 2 - (n - 1.0) ** 2) / (1.0 - r), 0.0))
    return n + 1j * k


def airy(r12, r23, path, nu):
    z = np.exp(1j * path[..., None] * nu)
    q = r23[..., None] * z
    return np.abs(r12[..., None] + q) ** 2 / np.abs(1.0 + r12[..., None] * q) ** 2


def dielectric_rows(cosine, thickness, film, substrate, nu):
    rs12, rp12, cos_f = amplitudes(cosine, 1.0, film)
    rs23, rp23, _ = amplitudes(cos_f, film, substrate)
    path = 4.0 * np.pi * film * thickness * cos_f
    return 0.5 * (airy(rs12, rs23 + 0j, path, nu) + airy(rp12, rp23 + 0j, path, nu))


def conductor_rows(cosine, thickness, film, f0, nu):
    rs12, rp12, cos_f = amplitudes(cosine, 1.0, film)
    metal = conductor_ior(f0)
    u = np.sqrt(metal * metal - film * film * (1.0 - cos_f * cos_f) + 0j)
    rs23 = (film * cos_f - u) / (film * cos_f + u)
    rp23 = (metal * metal * cos_f - film * u) / (metal * metal * cos_f + film * u)
    path = 4.0 * np.pi * film * thickness * cos_f
    return 0.5 * (airy(rs12, rs23, path, nu) + airy(rp12, rp23, path, nu))


def srgb_slope(value):
    v = np.clip(value, 0.003, 1.0)
    return 255.0 * 1.055 / 2.4 * v ** (1.0 / 2.4 - 1.0)


def density(nu):
    per_nu = TRACER / 5.0 * TRACER_WAVES[:, None] ** 2
    lam = 1.0 / nu
    rows = np.stack([np.interp(lam, TRACER_WAVES, per_nu[:, c]) for c in range(3)], axis=1)
    return rows / rows.sum(axis=0)


def fit(samples=SAMPLES):
    nu = waves(samples)
    tracer_nu = 1.0 / TRACER_WAVES
    rows = [[], [], []]
    targets = [[], [], []]
    cosine, thickness = np.meshgrid(np.linspace(LEAST_COSINE, 1.0, 14), np.linspace(0.0, DIELECTRIC_REACH, 241))
    for film, substrate in DIELECTRICS:
        ours = dielectric_rows(cosine, thickness, film, substrate, nu).reshape(-1, samples)
        theirs = dielectric_rows(cosine, thickness, film, substrate, tracer_nu).reshape(-1, 81) @ TRACER
        for c in range(3):
            rows[c].append(ours)
            targets[c].append(theirs[:, c])
    cosine, thickness = np.meshgrid(np.linspace(LEAST_COSINE, 1.0, 10), np.linspace(0.0, METAL_REACH, 161))
    for f0 in METALS:
        for film in METAL_FILMS:
            for c in range(3):
                ours = conductor_rows(cosine, thickness, film, f0[c], nu).reshape(-1, samples)
                theirs = conductor_rows(cosine, thickness, film, f0[c], tracer_nu).reshape(-1, 81) @ TRACER[:, c]
                rows[c].append(ours)
                targets[c].append(theirs)
    prior = density(nu)
    out = np.zeros((samples, 3))
    for c in range(3):
        a = np.vstack(rows[c])
        t = np.concatenate(targets[c])
        scale = srgb_slope(t)
        m = np.vstack([a * scale[:, None], RIDGE * np.eye(samples), 1e4 * np.ones((1, samples))])
        b = np.concatenate([t * scale, RIDGE * prior[:, c], [1e4]])
        out[:, c] = np.linalg.lstsq(m, b, rcond=None)[0]
    return nu, out


def srgb(value):
    v = np.clip(value, 0.0, 1.0)
    return 255.0 * np.where(v <= 0.0031308, 12.92 * v, 1.055 * v ** (1.0 / 2.4) - 0.055)


def three_waves(cosine, thickness, film, substrate):
    sine = np.sqrt(1.0 - cosine * cosine)
    inner = np.sqrt(np.maximum(1.0 - (sine / film) ** 2, 0.0))
    r12 = ((cosine - film * inner) / (cosine + film * inner))[..., None]
    r23 = (film - substrate) / (film + substrate)
    c = np.cos(4.0 * np.pi * film * thickness[..., None] * inner[..., None] / np.array([640.0, 540.0, 455.0]))
    return np.clip((r12**2 + r23**2 + 2 * r12 * r23 * c) / (1 + (r12 * r23) ** 2 + 2 * r12 * r23 * c), 0.0, 1.0)


def measure(samples):
    nu, table = fit(samples)
    tracer_nu = 1.0 / TRACER_WAVES
    cosines = np.cos(np.radians(np.arange(0, 71, 5)))
    cosine, thickness = np.meshgrid(cosines, np.arange(0.0, 1001.0, 5.0))
    dielectric = max(
        np.abs(
            srgb(dielectric_rows(cosine, thickness, f, s, nu) @ table)
            - srgb(dielectric_rows(cosine, thickness, f, s, tracer_nu) @ TRACER)
        ).max()
        for f, s in DIELECTRICS
    )
    cosine, thickness = np.meshgrid(cosines, np.arange(0.0, 401.0, 5.0))
    conductor = max(
        np.abs(
            srgb(conductor_rows(cosine, thickness, f, f0[c], nu) @ table[:, c])
            - srgb(conductor_rows(cosine, thickness, f, f0[c], tracer_nu) @ TRACER[:, c])
        ).max()
        for f0 in METALS
        for f in METAL_FILMS
        for c in range(3)
    )
    cosine, thickness = np.meshgrid(cosines, np.arange(900.0, 1101.0, 5.0))
    steps = {"live": 0.0, "tracer": 0.0, "three": 0.0}
    for f, s in DIELECTRICS:
        for name, values in (
            ("live", dielectric_rows(cosine, thickness, f, s, nu) @ table),
            ("tracer", dielectric_rows(cosine, thickness, f, s, tracer_nu) @ TRACER),
            ("three", three_waves(cosine, thickness, f, s)),
        ):
            steps[name] = max(steps[name], np.abs(np.diff(srgb(values), axis=0)).max())
    return dielectric, conductor, steps


if __name__ == "__main__":
    if "--measure" in sys.argv:
        print("wavelengths,dielectric_to_1um,conductor_to_400nm,step_at_1um,tracer_step,three_wave_step")
        for samples in (8, 12, 16, 24):
            dielectric, conductor, steps = measure(samples)
            print(f"{samples},{dielectric:.2f},{conductor:.2f},{steps['live']:.2f},{steps['tracer']:.2f},{steps['three']:.2f}")
        sys.exit(0)
    nu, table = fit()
    step = nu[1] - nu[0]
    if "--rust" in sys.argv:
        body = ",\n".join("    [%s]" % ", ".join(literal(v) for v in w) for w in table)
        print(f"pub const FILM_SAMPLES: usize = {SAMPLES};")
        print(f"pub const FILM_NU: f32 = {literal(nu[0])};")
        print(f"pub const FILM_STEP: f32 = {literal(step)};")
        print(f"pub const FILM_WEIGHTS: [[f32; 3]; {SAMPLES}] = [\n{body},\n];")
    else:
        body = ",\n".join("    vec3f(%s)" % ", ".join(literal(v) for v in w) for w in table)
        print(f"const FILM_SAMPLES: u32 = {SAMPLES}u;")
        print(f"const FILM_NU: f32 = {literal(nu[0])};")
        print(f"const FILM_STEP: f32 = {literal(step)};")
        print(f"const FILM_WEIGHTS = array<vec3f, {SAMPLES}>(\n{body},\n);")
