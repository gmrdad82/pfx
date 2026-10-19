const SAME_POSITION: f64 = 1e-5;
const SAME_PLACE: f64 = 1e-6;
const SAME_NORMAL: f64 = 1e-4;
const FLAT: f64 = 1e-6;

#[derive(Clone, Copy, Debug)]
pub struct Shape<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
}

#[derive(Clone, Copy, Debug)]
pub struct Anchors {
    points: [usize; 4],
    solve: [[f64; 4]; 4],
    planar: bool,
}

pub type Rows = [[f64; 4]; 3];

pub fn key(count: usize, indices: &[u32], uvs: &[[f32; 2]]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |value: u32| {
        hash = (hash ^ u64::from(value)).wrapping_mul(0x0000_0100_0000_01b3);
    };
    eat(count as u32);
    for &index in indices {
        eat(index);
    }
    for uv in uvs {
        eat(uv[0].to_bits());
        eat(uv[1].to_bits());
    }
    hash
}

fn wide(point: [f32; 3]) -> [f64; 3] {
    point.map(f64::from)
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length3(a: [f64; 3]) -> f64 {
    dot3(a, a).sqrt()
}

fn inverse4(m: [[f64; 4]; 4]) -> Option<[[f64; 4]; 4]> {
    let mut a = m;
    let mut out = [[0.0; 4]; 4];
    for (k, row) in out.iter_mut().enumerate() {
        row[k] = 1.0;
    }
    for column in 0..4 {
        let pivot =
            (column..4).max_by(|&x, &y| a[x][column].abs().total_cmp(&a[y][column].abs()))?;
        if a[pivot][column].abs() < 1e-300 {
            return None;
        }
        a.swap(column, pivot);
        out.swap(column, pivot);
        let scale = 1.0 / a[column][column];
        for k in 0..4 {
            a[column][k] *= scale;
            out[column][k] *= scale;
        }
        for row in 0..4 {
            if row != column {
                let factor = a[row][column];
                for k in 0..4 {
                    a[row][k] -= factor * a[column][k];
                    out[row][k] -= factor * out[column][k];
                }
            }
        }
    }
    Some(out)
}

fn corners(positions: &[[f32; 3]], points: [usize; 4], planar: bool) -> [[f64; 3]; 4] {
    let p = points.map(|at| wide(positions[at]));
    if !planar {
        return p;
    }
    let normal = cross3(minus(p[1], p[0]), minus(p[2], p[0]));
    let size = length3(minus(p[1], p[0]));
    let unit = length3(normal).max(f64::MIN_POSITIVE);
    let top = std::array::from_fn(|k| p[0][k] + normal[k] / unit * size);
    [p[0], p[1], p[2], top]
}

pub fn anchors(positions: &[[f32; 3]]) -> Option<Anchors> {
    if positions.len() < 3 {
        return None;
    }
    let p = |at: usize| wide(positions[at]);
    let farthest = |score: &dyn Fn([f64; 3]) -> f64| {
        (0..positions.len())
            .map(|at| (at, score(p(at))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(at, _)| at)
    };
    let first = 0;
    let second = farthest(&|q| length3(minus(q, p(first))));
    let axis = minus(p(second), p(first));
    let size = length3(axis);
    if size <= 0.0 {
        return None;
    }
    let third = farthest(&|q| length3(cross3(axis, minus(q, p(first)))));
    let normal = cross3(axis, minus(p(third), p(first)));
    if length3(normal) <= size * size * FLAT {
        return None;
    }
    let last = farthest(&|q| dot3(normal, minus(q, p(first))).abs());
    let height = dot3(normal, minus(p(last), p(first))).abs() / length3(normal);
    let planar = height <= size * FLAT;
    let points = [first, second, third, last];
    let rows = corners(positions, points, planar).map(|q| [q[0], q[1], q[2], 1.0]);
    Some(Anchors {
        points,
        solve: inverse4(rows)?,
        planar,
    })
}

pub fn register(lead: Shape<'_>, anchors: &Anchors, piece: Shape<'_>) -> Option<Rows> {
    if lead.positions.len() != piece.positions.len() || lead.normals.len() != piece.normals.len() {
        return None;
    }
    let targets = corners(piece.positions, anchors.points, anchors.planar);
    let mut rows = [[0.0f64; 4]; 3];
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, value) in row.iter_mut().enumerate() {
            *value = (0..4).map(|i| anchors.solve[c][i] * targets[i][r]).sum();
        }
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for point in piece.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(f64::from(point[k]));
            hi[k] = hi[k].max(f64::from(point[k]));
        }
    }
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    let reach = (0..3)
        .map(|k| lo[k].abs().max(hi[k].abs()))
        .fold(0.0, f64::max);
    let tolerance = SAME_POSITION * extent + SAME_PLACE * reach;
    for (from, to) in lead.positions.iter().zip(piece.positions) {
        let p = wide(*from);
        for (r, row) in rows.iter().enumerate() {
            let mapped = row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3];
            if (mapped - f64::from(to[r])).abs() > tolerance {
                return None;
            }
        }
    }
    let m = |r: usize, c: usize| rows[r][c];
    let cofactor = |r: usize, c: usize| {
        let (r0, r1) = ((r + 1) % 3, (r + 2) % 3);
        let (c0, c1) = ((c + 1) % 3, (c + 2) % 3);
        m(r0, c0) * m(r1, c1) - m(r0, c1) * m(r1, c0)
    };
    let determinant: f64 = (0..3).map(|c| m(0, c) * cofactor(0, c)).sum();
    if determinant.is_nan() || determinant == 0.0 {
        return None;
    }
    let sign = determinant.signum();
    for (from, to) in lead.normals.iter().zip(piece.normals) {
        let n = wide(*from);
        let turned: [f64; 3] =
            std::array::from_fn(|r| (0..3).map(|c| cofactor(r, c) * n[c]).sum::<f64>() * sign);
        let size = length3(turned);
        if size <= 0.0 {
            if length3(wide(*to)) > 0.0 {
                return None;
            }
            continue;
        }
        if dot3(turned.map(|v| v / size), wide(*to)) < 1.0 - SAME_NORMAL {
            return None;
        }
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_scaled_copy_registers_and_a_bent_one_does_not() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, 0.0, 3.0],
            [1.0, 1.0, 1.0],
        ];
        let normals = [[0.0, 0.0, 1.0]; 5];
        let turn = |p: [f32; 3]| [-2.0 * p[1] + 5.0, 2.0 * p[0] - 1.0, 2.0 * p[2] + 0.5];
        let moved: Vec<[f32; 3]> = positions.iter().map(|&p| turn(p)).collect();
        let lead = Shape {
            positions: &positions,
            normals: &normals,
        };
        let anchors = anchors(&positions).unwrap();
        let rows = register(
            lead,
            &anchors,
            Shape {
                positions: &moved,
                normals: &normals,
            },
        )
        .unwrap();
        let expected = [
            [0.0, -2.0, 0.0, 5.0],
            [2.0, 0.0, 0.0, -1.0],
            [0.0, 0.0, 2.0, 0.5],
        ];
        for (row, want) in rows.iter().zip(expected) {
            for (got, want) in row.iter().zip(want) {
                assert!((got - want).abs() < 1e-6, "{rows:?}");
            }
        }
        let mut bent = moved.clone();
        bent[4][0] += 0.01;
        let bent = Shape {
            positions: &bent,
            normals: &normals,
        };
        assert!(register(lead, &anchors, bent).is_none());
    }
}
