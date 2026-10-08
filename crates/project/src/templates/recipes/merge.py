import json, math, re, struct, sys, tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
MARK = Path(sys.argv[1])
WORDS = Path(sys.argv[2])
OUT = Path(sys.argv[3])


def read(path):
    b = path.read_bytes()
    jl = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20 : 20 + jl])
    bl = struct.unpack("<I", b[20 + jl : 24 + jl])[0]
    return j, b[28 + jl : 28 + jl + bl]


def accessor(j, binary, index):
    a = j["accessors"][index]
    v = j["bufferViews"][a["bufferView"]]
    start = v.get("byteOffset", 0) + a.get("byteOffset", 0)
    width = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
    code = {5126: "f", 5125: "I", 5123: "H"}[a["componentType"]]
    count = a["count"] * width
    flat = struct.unpack_from("<%d%s" % (count, code), binary, start)
    return [flat[k : k + width] for k in range(0, count, width)] if width > 1 else list(flat)


def primitives(path, pick, place):
    j, binary = read(path)
    names = [m["name"] for m in j["materials"]]
    out = {}
    for node in j["nodes"]:
        if "mesh" not in node:
            continue
        for p in j["meshes"][node["mesh"]]["primitives"]:
            group = pick(node.get("name", ""), names[p["material"]])
            if group is None:
                continue
            pos = [place(v) for v in accessor(j, binary, p["attributes"]["POSITION"])]
            nor = [place(v, normal=True) for v in accessor(j, binary, p["attributes"]["NORMAL"])]
            uv = accessor(j, binary, p["attributes"]["TEXCOORD_0"]) if "TEXCOORD_0" in p["attributes"] else [(0.0, 0.0)] * len(pos)
            idx = accessor(j, binary, p["indices"])
            slot = out.setdefault(group, ([], [], [], []))
            base = len(slot[0])
            slot[0].extend(pos)
            slot[1].extend(nor)
            slot[2].extend(uv)
            slot[3].extend(i + base for i in idx)
    return out


def write(path, node_name, groups, colours):
    blob = bytearray()
    views, accessors, prims, materials = [], [], [], []

    def add(data, code, target, kind, count, extra=None):
        while len(blob) % 4:
            blob.append(0)
        offset = len(blob)
        blob.extend(struct.pack("<%d%s" % (len(data), code), *data))
        views.append({"buffer": 0, "byteOffset": offset, "byteLength": len(blob) - offset, "target": target})
        a = {"bufferView": len(views) - 1, "componentType": 5126 if code == "f" else 5125, "count": count, "type": kind}
        if extra:
            a.update(extra)
        accessors.append(a)
        return len(accessors) - 1

    for name in sorted(groups):
        pos, nor, uv, idx = groups[name]
        lo = [min(v[k] for v in pos) for k in range(3)]
        hi = [max(v[k] for v in pos) for k in range(3)]
        p = add([c for v in pos for c in v], "f", 34962, "VEC3", len(pos), {"min": lo, "max": hi})
        n = add([c for v in nor for c in v], "f", 34962, "VEC3", len(nor))
        t = add([c for v in uv for c in v], "f", 34962, "VEC2", len(uv))
        i = add(idx, "I", 34963, "SCALAR", len(idx))
        materials.append({"name": name, "pbrMetallicRoughness": {"baseColorFactor": colours[name] + [1.0]}})
        prims.append({"attributes": {"POSITION": p, "NORMAL": n, "TEXCOORD_0": t}, "indices": i, "material": len(materials) - 1})
    doc = {
        "asset": {"version": "2.0", "generator": "pfx crates/project/src/templates/recipes/merge.py from manfred 0.16.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": node_name, "mesh": 0}],
        "meshes": [{"name": node_name, "primitives": prims}],
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(blob)}],
    }
    text = json.dumps(doc, separators=(",", ":")).encode()
    while len(text) % 4:
        text += b" "
    while len(blob) % 4:
        blob.append(0)
    total = 12 + 8 + len(text) + 8 + len(blob)
    data = struct.pack("<III", 0x46546C67, 2, total) + struct.pack("<II", len(text), 0x4E4F534A) + text + struct.pack("<II", len(blob), 0x004E4942) + bytes(blob)
    path.write_bytes(data)


def badge_group(node, material):
    if node == "ground":
        return None
    prefix = re.sub(r"_\d+$", "", material)
    if prefix in ("tile", "dome", "lime"):
        return prefix
    if prefix == "mesh":
        return "lines"
    if prefix == "rim":
        return "white"
    if prefix in ("rimband", "streak"):
        return "white" if material in BRIGHT else "violet"
    if prefix == "dust":
        return "violet"
    raise SystemExit(f"no group for {material}")


recipe = tomllib.loads((ROOT / "brand/recipes/incoming-full.toml").read_text())["materials"]
BRIGHT = {name for name, m in recipe.items() if sum(m["emission"]) / 3 > 0.7}


def stand(v, normal=False):
    x, y, z = v
    return (x, -z, y)


def keep(v, normal=False):
    return tuple(v)


badge = primitives(MARK, badge_group, stand)
words = primitives(WORDS, lambda node, material: "wordmark", keep)


def average(names, key):
    values = [recipe[n][key] for n in names]
    return [round(sum(v[k] for v in values) / len(values), 5) for k in range(3)]


members = {}
for name in recipe:
    group = badge_group("", name) if name != "ground_00" else None
    if group:
        members.setdefault(group, []).append(name)
library = {}
for group, names in sorted(members.items()):
    library[group] = {
        "family": "lacquer",
        "base": average(names, "base"),
        "roughness": round(sum(recipe[n]["roughness"] for n in names) / len(names), 3),
        "clearcoat": 0.4,
        "emission": average(names, "emission"),
    }
library["wordmark"] = {"family": "lacquer", "base": [0.84, 0.83, 0.83], "roughness": 0.3, "clearcoat": 0.4, "emission": [0.0, 0.0, 0.0]}


def bounds(groups):
    pts = [v for g in groups.values() for v in g[0]]
    return [min(v[k] for v in pts) for k in range(3)], [max(v[k] for v in pts) for k in range(3)]


def tile_front():
    j, binary = read(MARK)
    node = next(n for n in j["nodes"] if n.get("name") == "tile")
    zs = [stand(v)[2] for p in j["meshes"][node["mesh"]]["primitives"] for v in accessor(j, binary, p["attributes"]["POSITION"])]
    return max(zs)


blo, bhi = bounds(badge)
wlo, whi = bounds(words)
front = tile_front()
gap = 0.12
x_height = 0.33
badge_at = [0.0, 0.0, 0.0]
word_at = [bhi[0] + gap - wlo[0], (blo[1] + bhi[1]) / 2 - x_height / 2, front - whi[2]]
lo = [min(blo[k] + badge_at[k], wlo[k] + word_at[k]) for k in range(3)]
hi = [max(bhi[k] + badge_at[k], whi[k] + word_at[k]) for k in range(3)]
centre = [(lo[k] + hi[k]) / 2 for k in range(3)]
OUT.mkdir(parents=True, exist_ok=True)
write(OUT / "badge.glb", "badge", badge, {k: library[k]["base"] for k in badge})
write(OUT / "wordmark.glb", "wordmark", words, {"wordmark": library["wordmark"]["base"]})
lines = ["format = 1", ""]
for name, m in library.items():
    lines.append(f"[materials.{name}]")
    lines.append(f'family = "{m["family"]}"')
    for key in ("base", "roughness", "clearcoat", "emission"):
        value = m[key]
        lines.append(f"{key} = {json.dumps(value)}" if isinstance(value, list) else f"{key} = {value}")
    lines.append("")
(OUT / "incoming.materials.toml").write_text("\n".join(lines))
rel = lambda at: [round(at[k] - centre[k], 4) for k in range(3)]
print(json.dumps({
    "badge_at": rel(badge_at),
    "wordmark_at": rel(word_at),
    "half_height": round((hi[1] - lo[1]) / 2, 4),
    "size": [round(hi[k] - lo[k], 4) for k in range(3)],
    "tile_front": round(front, 4),
    "wordmark_front": round(whi[2] + word_at[2], 4),
    "groups": {k: len(v[3]) // 3 for k, v in badge.items()},
}))
