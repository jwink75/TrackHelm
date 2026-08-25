// High-precision Robert Bristow-Johnson (RBJ) Audio EQ Biquad Filter

#[derive(Clone, Copy, Debug)]
pub enum FilterType {
    LowShelf,
    Peaking,
    HighShelf,
}

#[derive(Clone, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    // Per-channel state: (x1, x2, y1, y2)
    state: Vec<[f64; 4]>,
}

impl Biquad {
    pub fn new(channels: usize) -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            state: vec![[0.0; 4]; channels.max(1)],
        }
    }

    pub fn set_params(&mut self, filter_type: FilterType, sample_rate: f64, freq: f64, gain_db: f64, q: f64) {
        let a = 10.0f64.powf(gain_db / 40.0);
        let omega = 2.0 * std::f64::consts::PI * (freq.clamp(20.0, sample_rate * 0.49) / sample_rate);
        let sin_w = omega.sin();
        let cos_w = omega.cos();
        let alpha = sin_w / (2.0 * q.max(0.001));

        let (b0, b1, b2, a0, a1, a2) = match filter_type {
            FilterType::LowShelf => {
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha);
                let a0 = (a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha;
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w);
                let a2 = (a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::Peaking => {
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cos_w;
                let b2 = 1.0 - alpha * a;
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cos_w;
                let a2 = 1.0 - alpha / a;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::HighShelf => {
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha);
                let a0 = (a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha;
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w);
                let a2 = (a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha;
                (b0, b1, b2, a0, a1, a2)
            }
        };

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    pub fn process_sample(&mut self, ch: usize, input: f32) -> f32 {
        if ch >= self.state.len() {
            return input;
        }
        let x = input as f64;
        let s = &mut self.state[ch];
        let y = self.b0 * x + self.b1 * s[0] + self.b2 * s[1] - self.a1 * s[2] - self.a2 * s[3];

        s[1] = s[0];
        s[0] = x;
        s[3] = s[2];
        s[2] = y;

        y as f32
    }

    pub fn reset(&mut self) {
        for s in self.state.iter_mut() {
            *s = [0.0; 4];
        }
    }
}
