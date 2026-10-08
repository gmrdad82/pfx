import sys

import numpy as np

XYZ_TO_RGB = np.array(
    [
        [3.2404542, -1.5371385, -0.4985314],
        [-0.9692660, 1.8760108, 0.0415560],
        [0.0556434, -0.2040259, 1.0572252],
    ]
)
LOW = 380.0
STEP = 5.0
COUNT = 81
NODE_SCALE = 1.0 / 2.52


def lobe(x, mu, left, right):
    t = (x - mu) / np.where(x < mu, left, right)
    return np.exp(-0.5 * t * t)


def wyman(l):
    x = 1.056 * lobe(l, 599.8, 37.9, 31.0) + 0.362 * lobe(l, 442.0, 16.0, 26.7) - 0.065 * lobe(l, 501.1, 20.4, 26.2)
    y = 0.821 * lobe(l, 568.8, 46.9, 40.5) + 0.286 * lobe(l, 530.9, 16.3, 31.1)
    z = 1.217 * lobe(l, 437.0, 11.8, 36.0) + 0.681 * lobe(l, 459.0, 26.0, 13.8)
    return np.stack([x, y, z], axis=1) @ XYZ_TO_RGB.T


def literal(value):
    text = str(np.float32(value))
    if "." not in text and "e" not in text:
        text += ".0"
    return text


def weights(probe="crates/trace/film/wavelength.csv"):
    table = np.loadtxt(probe, delimiter=",")
    waves = LOW + STEP * np.arange(COUNT)
    node = np.stack([np.interp(waves, table[:, 0], table[:, 1 + c]) for c in range(3)], axis=1) / NODE_SCALE
    fit = wyman(waves)
    rows = np.where(node > 0.0, node, np.minimum(fit, 0.0))
    return rows / rows.sum(axis=0)


if __name__ == "__main__":
    rows = weights()
    if "--rust" in sys.argv:
        body = ",\n".join("    [%s]" % ", ".join(literal(v) for v in w) for w in rows)
        print(f"pub const WEIGHTS: [[f32; 3]; {COUNT}] = [\n{body},\n];")
    else:
        body = ",\n".join("    vec3f(%s)" % ", ".join(literal(v) for v in w) for w in rows)
        print(f"const CF_WEIGHTS = array<vec3f, {COUNT}>(\n{body},\n);")
