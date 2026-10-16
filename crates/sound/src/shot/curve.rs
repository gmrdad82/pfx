#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    Linear,
    Smooth,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub name: String,
    pub keys: Vec<[f32; 2]>,
    pub ease: Ease,
}

pub(crate) struct Track {
    keys: Vec<[f32; 2]>,
    slopes: Vec<f32>,
    ease: Ease,
}

impl Track {
    pub(crate) fn new(curve: &Curve) -> Self {
        let keys = curve.keys.clone();
        let slopes = match curve.ease {
            Ease::Linear => Vec::new(),
            Ease::Smooth => monotone_slopes(&keys),
        };
        Self {
            keys,
            slopes,
            ease: curve.ease,
        }
    }

    pub(crate) fn at(&self, t: f32) -> f32 {
        let keys = &self.keys;
        let last = keys.len() - 1;
        if t <= keys[0][0] {
            return keys[0][1];
        }
        if t >= keys[last][0] {
            return keys[last][1];
        }
        let i = keys.partition_point(|k| k[0] <= t) - 1;
        let [t0, v0] = keys[i];
        let [t1, v1] = keys[i + 1];
        let h = t1 - t0;
        if h <= 0.0 {
            return v1;
        }
        let s = (t - t0) / h;
        match self.ease {
            Ease::Linear => v0 + (v1 - v0) * s,
            Ease::Smooth => {
                let s2 = s * s;
                let s3 = s2 * s;
                let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
                let h10 = s3 - 2.0 * s2 + s;
                let h01 = -2.0 * s3 + 3.0 * s2;
                let h11 = s3 - s2;
                h00 * v0 + h10 * h * self.slopes[i] + h01 * v1 + h11 * h * self.slopes[i + 1]
            }
        }
    }
}

fn monotone_slopes(keys: &[[f32; 2]]) -> Vec<f32> {
    let n = keys.len();
    if n < 2 {
        return vec![0.0; n];
    }
    let delta: Vec<f32> = keys
        .windows(2)
        .map(|w| {
            let h = w[1][0] - w[0][0];
            if h > 0.0 {
                (w[1][1] - w[0][1]) / h
            } else {
                0.0
            }
        })
        .collect();
    let mut m = vec![0.0; n];
    m[0] = delta[0];
    m[n - 1] = delta[n - 2];
    for k in 1..n - 1 {
        m[k] = if delta[k - 1] * delta[k] <= 0.0 {
            0.0
        } else {
            (delta[k - 1] + delta[k]) * 0.5
        };
    }
    for k in 0..n - 1 {
        if delta[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
            continue;
        }
        let a = m[k] / delta[k];
        let b = m[k + 1] / delta[k];
        let sum = a * a + b * b;
        if sum > 9.0 {
            let tau = 3.0 / pfx_core::sim::sqrtf(sum);
            m[k] = tau * a * delta[k];
            m[k + 1] = tau * b * delta[k];
        }
    }
    m
}
