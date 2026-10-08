import re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
OUT = Path(sys.argv[1])
S = 0.00065
BASELINE = 1000.0
LEFT = 61.0


def fmt(v):
    s = f"{v:.5f}".rstrip("0")
    return s + "0" if s.endswith(".") else s


def flatten(d):
    tokens = re.findall(r"[MLQVHZ]|-?\d+(?:\.\d+)?", d)
    contours, cur, i, cmd, pos = [], [], 0, None, (0.0, 0.0)
    while i < len(tokens):
        t = tokens[i]
        if t in "MLQVHZ":
            cmd = t
            i += 1
            if cmd == "Z":
                if cur:
                    contours.append(cur)
                cur = []
            continue
        if cmd == "M":
            if cur:
                contours.append(cur)
            pos = (float(tokens[i]), float(tokens[i + 1]))
            cur = [pos]
            i += 2
            cmd = "L"
        elif cmd == "L":
            pos = (float(tokens[i]), float(tokens[i + 1]))
            cur.append(pos)
            i += 2
        elif cmd == "H":
            pos = (float(tokens[i]), pos[1])
            cur.append(pos)
            i += 1
        elif cmd == "V":
            pos = (pos[0], float(tokens[i]))
            cur.append(pos)
            i += 1
        elif cmd == "Q":
            c = (float(tokens[i]), float(tokens[i + 1]))
            e = (float(tokens[i + 2]), float(tokens[i + 3]))
            for k in range(1, 9):
                u = k / 8
                a = (1 - u) ** 2
                b = 2 * (1 - u) * u
                cc = u * u
                cur.append((a * pos[0] + b * c[0] + cc * e[0], a * pos[1] + b * c[1] + cc * e[1]))
            pos = e
            i += 4
    if cur:
        contours.append(cur)
    out = []
    for contour in contours:
        clean = []
        for p in contour:
            if not clean or abs(p[0] - clean[-1][0]) + abs(p[1] - clean[-1][1]) > 1e-6:
                clean.append(p)
        if len(clean) > 2 and abs(clean[0][0] - clean[-1][0]) + abs(clean[0][1] - clean[-1][1]) < 1e-6:
            clean.pop()
        out.append(clean)
    return out


def area(c):
    return sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(c, c[1:] + c[:1])) / 2


def points(c):
    return "{ points = [" + ", ".join(f"[{fmt((x - LEFT) * S)}, {fmt((BASELINE - y) * S)}]" for x, y in c) + "] }"


def wordmark():
    svg = (ROOT / "assets/marks/wordmark.svg").read_text()
    lines = [
        "# pfx's wordmark in 3D: assets/marks/wordmark.svg extruded, the glyphs' quadratic curves in eight steps, 0.00065 m per SVG unit, baseline at y 0; build with manfred build.",
        "manfred = 1",
        'name = "pfx-wordmark"',
        "seed = 7",
    ]
    letters = ["p", "f", "x"]
    for letter, d in zip(letters, re.findall(r' d="([^"]+)"', svg)):
        contours = sorted(flatten(d), key=lambda c: -abs(area(c)))
        outline = points(contours[0])
        if len(contours) > 1:
            outline = "{ difference = [" + ", ".join([outline] + [points(hole) for hole in contours[1:]]) + "] }"
        lines += [
            "",
            "[[object]]",
            f'name = "{letter}"',
            'kind = "outline"',
            f"outline = {outline}",
            "front = 0.07",
            "back = 0.0",
            'profile = { kind = "round", width = 0.008, segments = 6 }',
            'material = "wordmark"',
            "smooth_angle = 40.0",
        ]
    lines += [
        "",
        "[materials.wordmark]",
        'family = "lacquer"',
        "base = [0.84, 0.83, 0.83]",
        "roughness = 0.3",
        "clearcoat = 0.4",
    ]
    return "\n".join(lines) + "\n"


def grid():
    lines = [
        "# A floor grid of one metre squares, ten by ten, with the X axis in red and the Z axis in blue through the origin; build with manfred build.",
        "manfred = 1",
        'name = "pfx-grid"',
        "seed = 7",
    ]
    for k in range(-5, 6):
        if k == 0:
            continue
        lines += ["", "[[object]]", f'name = "x{k:+d}"', 'kind = "plane"', f"at = [{fmt(float(k))}, 0.0, 0.0]", "size = [0.02, 10.0]", 'material = "grid"']
        lines += ["", "[[object]]", f'name = "z{k:+d}"', 'kind = "plane"', f"at = [0.0, 0.0, {fmt(float(k))}]", "size = [10.0, 0.02]", 'material = "grid"']
    lines += ["", "[[object]]", 'name = "axis_x"', 'kind = "plane"', "at = [0.0, 0.001, 0.0]", "size = [10.0, 0.035]", 'material = "axis_x"']
    lines += ["", "[[object]]", 'name = "axis_z"', 'kind = "plane"', "at = [0.0, 0.001, 0.0]", "size = [0.035, 10.0]", 'material = "axis_z"']
    lines += [
        "",
        "[materials.grid]",
        "base = [0.05, 0.05, 0.055]",
        "roughness = 0.9",
        "",
        "[materials.axis_x]",
        "base = [0.6, 0.05, 0.07]",
        "roughness = 0.6",
        "emission = [0.5, 0.03, 0.05]",
        "",
        "[materials.axis_z]",
        "base = [0.08, 0.2, 0.7]",
        "roughness = 0.6",
        "emission = [0.05, 0.15, 0.6]",
    ]
    return "\n".join(lines) + "\n"


OUT.mkdir(parents=True, exist_ok=True)
(OUT / "wordmark.toml").write_text(wordmark())
(OUT / "grid.toml").write_text(grid())
