# Asset geometry

Status: 2026-10-06. `crates/geom` builds the solids the asset renders need, on the CPU, without Blender: an SVG mark becomes extruded, bevelled solids, and an icon's parts fuse into one smooth shape through a distance field. Every parameter and default follows `crates/assets/blender/render.py`, so `crates/assets` can map its preset keys one to one and stage the meshes into `pfx_trace`.

Three modules:
- `svg`: reads a master (`Drawing::parse`) into elements in document order, with flattened paths, paints, stroke widths, fill rules, mask holes and the ground rect.
- `mark`: `Mark::build(svg, &Form)` turns the drawing into solids.
- `icon`: `Icon::build()` turns parts into pieces, fused or apart.

`field::Surface` is the mesher both use for anything round.

Every mesh is a `pfx_geom::mesh::Mesh` with positions, unit normals, tangents (computed from the UVs), UVs and `u32` indices. Those are the fields of `pfx_load::mesh::Primitive` (with `uvs1: None`) and of `pfx_trace::Mesh`, so a caller passes them on as they are. Triangles wind counter-clockwise seen from outside.

## Coordinates

A mark is centred on its viewBox, with y up and the viewBox's longest side 1 unit long. `Mark::scale` is world units per SVG unit (one over that side). Depth, bevel, layer and lift are in those world units, as in render.py, where they're multiples of the master's size. Inset and stroke width stay in SVG units, as render.py reads them. UVs map the viewBox: u runs left to right and v top to bottom, on the caps, bevels and walls alike. That gives a gradient or decal the master's own layout. A gradient solid also carries its `span`, the world x of the gradient's `x1` and `x2`, which render.py hands its material.

An icon stays in its spec's units, the box of about ±1 that icon specs use. Its UVs are a planar xy projection over the piece's bounds.

## SVG masters

`Drawing::parse` follows render.py's reading of a master:
- **Refusals:** an SVG with a DOCTYPE or ENTITY. roxmltree also refuses any DTD.
- **viewBox:** `0 0 100 100` when none is given.
- **Elements:** `path`, `rect`, `circle`, `ellipse`, `polygon`, `polyline` and `line`, counted in document order. Nothing inside `defs`, `mask`, `clipPath`, gradients, `title`, `desc`, `symbol`, `pattern`, `marker` or `metadata` counts.
- **Style:** inherited down the tree from the attributes `fill`, `stroke`, `stroke-width`, `stroke-linecap`, `stroke-linejoin`, `mask`, `fill-rule` and `opacity`, and from `style="…"`. `fill` starts black.
- **Paint:** `none`, `transparent` and empty mean no paint. Any CSS colour parses to sRGB bytes, and `url(#id)` names a gradient with its stops and `x1`, `x2`, `y1` and `y2`. A missing gradient is an error.
- **Transforms:** composed down the tree. A stroke width is scaled by √|det|, as render.py's `spread` does.
- **Ground:** a rect covering at least 98% of a square viewBox is the ground. It leaves the solids, its fill becomes `Mark::ground`, and it isn't counted as an element. A non-square viewBox never has a ground.
- **Mask holes:** in a mask, rects filled `black`, `#000` or `#000000` are holes (with `rx`) cut from the elements that use the mask, in root coordinates.
- **Flattening:** curves flatten adaptively, Béziers by Wang's bound and arcs and circles by chord error, to `Form::tolerance` × the viewBox's longest side (default 2e-4, a fifth of a pixel at 1024).

## Marks

### Form

| `Form` field | render.py | default |
| --- | --- | --- |
| `depth` | `form.depth`, the half thickness: the curve extrudes from −depth to +depth | 0.12 |
| `bevel` | `form.bevel`, capped at 0.2 × the element's larger side | 0.05 |
| `segments` | `form.segments`, Blender's bevel resolution: a quarter round of `segments + 1` facets | 8 |
| `profile` | `Round` (render.py's only mode) or `Chamfer`, one 45° facet | `Round` |
| `layer` | `form.layer`: each new paint sits `layer × depth` higher than the one before | 0.25 |
| `inset` | `form.inset`, all elements or per element (SVG units) | 0 |
| `element_depth` | per-element z scale, as Blender's object scale | 1 |
| `element_z` | per-element lift, in units of depth | 0 |
| `element_material` | per-element look; it splits the paint's solid | none |
| `merge` | false keeps same-paint elements apart | true |
| `smooth_angle` | the 40° auto-smooth angle of `shade_auto_smooth` | 40 |
| `tolerance` | curve flattening, a fraction of the viewBox | 2e-4 |
| `relax` | relaxation passes for strokes and pillows | 2 |

`Each { all, by_element }` covers render.py's `per`: a scalar for every element, or a list or map by element number.

### Fills

Each fill is cleaned by its fill rule into outer contours and holes (i_overlay, on an i64 grid). Then:
- **Inset:** it's offset inward by `inset` with mitred corners.
- **Mask holes:** they're subtracted.
- **Merge:** fills of one paint, material, depth and lift are unioned in 2D.

Every outline then becomes a solid with the round bevel Blender gives a 2D curve with extrude and bevel depth:
- **Wall:** a vertical wall from −depth to +depth at the outline grown by the bevel.
- **Bevel:** a quarter round on each side that comes back to the outline at ±(depth + bevel).
- **Caps:** flat caps over the outline, triangulated with their holes by lyon.

The outline grows with mitred corners, as Blender's bevel does, capped at four times the bevel on hairpins.

Normals follow auto smooth at `smooth_angle`:
- The caps are flat, (0, 0, ±1) exactly.
- Along the profile, a vertex is smooth where its two facets meet within the angle, using the round's own analytic normal there. The default round is smooth from the wall through the bevel into the cap, which it meets tangentially.
- Around the outline, a corner sharper than the angle splits its normals, so a rectangle keeps crisp vertical edges and a flattened circle stays smooth.
- A chamfer splits everywhere, so each facet is flat.
- Per-element depth scales z, and normals take the inverse transpose.

### Strokes and pillows

These follow render.py's two inflated cases, through a distance field (below):
- **A stroke** becomes a round tube of radius `max(width / 2 − inset, width × 0.1)` around the stroke's centre line, z-centred like Blender's 3D curve. Open ends are round, and closed paths close.
- **A fill stroked in its own paint** (render.py compares the two strings) becomes a pillow: the fill region extruded to ±`0.15 × grow`, then inflated by the rest of `grow = max(width / 2 − inset, width × 0.1)`.

Both are meshed at render.py's voxel, `max(radius / 10, 1 / 1400)`, through a 2D distance raster at half that size. Mask holes cut them as prisms through z.

### Solids

Pieces are grouped as render.py's `merge_same_paint` does: one solid per paint (plus material, plus element when `merge` is false), in the order paints first appear.

Within a solid, each element keeps its own bevel, as render.py's 3D union of separately bevelled elements did: filled regions of the same bevel, depth and lift join in 2D and are extruded together, and regions of different bevels are extruded on their own and joined into the solid's mesh, so a large shape keeps its bevel beside a small one and touching elements of different sizes keep the step between them.

Every paint's solids are lifted by `order × layer × depth`, then by the element's `element_z × depth`. `Solid` carries the paint, the material, that order, the element numbers and the gradient span.

## Icons

### Parts and defaults

| part | fields and defaults | objects in render.py |
| --- | --- | --- |
| `Ball` | `at`, `z`, `r` 0.3 | 1 |
| `Drop` | `at`, `z`, `r` 1.0, `squash` 0.45, `stretch` 1.0: an ellipsoid of radii r, r × stretch, r × squash | 1 |
| `Capsule` | `from`, `to`, `z`, `r` 0.08 | 3 (tube and two balls) |
| `Tube` | `points`, `z`, `r` 0.08 | 3 |
| `Torus` | `at`, `z`, `major` 0.6, `minor` 0.1, in the xy plane | 1 |
| `Box` | `at`, `z`, `half` [0.3, 0.3, 0.1], `round` 0.05 (capped at 0.9 × the smallest half), `angle` 0 (degrees about z) | 1 |

`PartShape::ball()`, `drop()`, `capsule(from, to)`, `tube(points)`, `torus()` and `cube()` give those defaults.

### Fusing

Parts group by `material`, in first-appearance order, as render.py's `groups` do. A group is fused when `fuse` is true (the default) and it holds more than one Blender object, so a lone capsule or tube is fused too, as in render.py.

A fused group is one signed distance field:
- **Union:** the parts' exact distances (a drop's is the scaled sphere's, a lower bound with the exact zero set), joined by a polynomial smooth minimum with blend radius `blend`.
- **Blend:** render.py's union is a hard join, remeshed and then smoothed by 8 corrective passes at half strength. That rounds the crease over about two voxels, so `blend` defaults to 2 × the voxel. `blend = Some(0.0)` gives the hard union.
- **Voxel:** `voxel` defaults to 0.01 and is never finer than 1/900 of the group's longest side, render.py's cap.
- **Mesher:** meshed at that voxel with 8 relaxation passes.

A large `voxel` gives render.py's blocky look: at 0.1 a ball of radius 0.4 is eight cells across, faceted but closed.

Unfused parts stay separate pieces, one analytic mesh each from `shapes::Shape` at a chord error of 0.001 (`Icon::error`), comparable to Blender's 48 × 24 UV sphere. A tube is one capsule per segment, overlapping at the joints.

`Piece` carries the material, the part indices and the mesh.

## The distance-field mesher

`field::Surface { voxel, relax }.mesh(&field, lo, hi)` meshes the zero set of any function that is at most about 1.5-Lipschitz:
- **Sparse grid:** blocks of 8³ cells, padded two voxels past the bounds. A block whose centre lies farther from the surface than 1.5 × its half diagonal (plus a cell) is skipped, since no zero can lie in it.
- **Surface nets:** one vertex per cell the surface crosses, at the mean of its edge crossings, and one quad per crossed grid edge, wound outward.
- **Projection:** each vertex steps onto the surface by Newton steps along the gradient (central differences at a tenth of a voxel), never more than a voxel from where it began.
- **Relaxation:** `relax` passes move each vertex halfway to its neighbours' mean, then back onto the surface.
- **Triangles:** quads split along the shorter diagonal.
- **Normals:** the field's normalised gradient.

The result is closed and oriented, and its vertices lie on the surface within about 2% of a voxel. Without an iso tolerance there's no shrinkage, so volumes match analytic ones within a few tenths of a percent at the default voxel.

## Determinism

Everything is single-threaded and ordered:
- maps are `BTreeMap`s, or `HashMap`s used only for lookups;
- groups keep first-appearance order;
- relaxation is Jacobi.

The same input gives the same bytes (`Mesh::bytes`) on every run and every thread. The tests check this.

## Triangle counts at the defaults

| input | triangles |
| --- | ---: |
| the sample mark (`crates/assets/tests/recipes`' disc and bar) | 3,876 + 156 |
| icon `dot` (one ball, r 0.4, unfused) | 3,906 |
| icon `ring` (torus 0.5 / 0.1, unfused) | 4,928 |
| two fused balls (r 0.3, 0.4 apart) | 55,996 |

## Speed

Measured on the Ryzen 9 9950X, single-threaded, release, best of five, with `cargo run --release -p pfx-geom --example asset_speed`:

| input | ms | triangles |
| --- | ---: | ---: |
| mark: the sample | 0.37 | 4,032 |
| mark: a 24-tooth gear with an even-odd hole, a stroked S curve and a pillow | 89 | 192,348 |
| icon `dot` / `ring` (unfused) | 0.3 | 3,906 / 4,928 |
| icon: two fused balls | 64 | 55,996 |
| icon: three balls, a tube and a box, fused | 238 | 81,628 |
| icon: a default drop (r 1) and a capsule, fused | 261 | 238,476 |

A mark or icon costs well under a second, against a render's seconds of tracing, so geometry is never the long pole.

## Tests

`crates/geom/src/assets_tests.rs`:
- **Volume:**
  - a bevelled rectangle to 1e-4 of the exact faceted-profile volume, with even-odd and mask holes as well;
  - a disc to 0.2%;
  - a stroke ring against the torus, a pillow against its Steiner volume, and fused balls against the union of two spheres;
  - a lone capsule, a rotated rounded box against Steiner's formula, and an unfused drop against its ellipsoid.
- **Silhouette:** the bevelled disc against π(r + bevel)², and a meshed torus against its annulus, by rasterising the xy projection.
- **Watertightness:** every directed edge is matched by its reverse once positions are welded.
- **Normals:** unit length, flat caps exactly (0, 0, ±1), a horizontal wall, split corners and chamfer facets, every face agreeing with its vertex normals, and field normals within 2° of the analytic gradient.
- **The rest:** determinism, triangle counts, layering, merge and material splits, per-element depth, lift and inset, the ground rect, flattening, and refusals.
