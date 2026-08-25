// High-precision Feedforward Dynamic Compressor with Soft Knee & Adaptive Ballistics

#[derive(Clone, Debug)]
pub struct Compressor {
    sample_rate: f64,
    threshold_db: f32,
    ratio: f32,
    makeup_db: f32,
    attack_ms: f32,
    release_ms: f32,
    knee_db: f32,
    envelope_db: f64,
    ga: f64, // Attack coeff
    gr: f64, // Release coeff
}

impl Compressor {
    pub fn new(sample_rate: f64) -> Self {
        let mut comp = Self {
            sample_rate: sample_rate.max(1000.0),
            threshold_db: 0.0,
            ratio: 1.0,
            makeup_db: 0.0,
            attack_ms: 30.0,
            release_ms: 300.0,
            knee_db: 3.0,
            envelope_db: -120.0,
            ga: 0.0,
            gr: 0.0,
        };
        comp.update_coeffs();
        comp
    }

    pub fn set_params(
        &mut self,
        sample_rate: f64,
        threshold_db: f32,
        ratio: f32,
        makeup_db: f32,
        attack_ms: f32,
        release_ms: f32,
    ) {
        self.sample_rate = sample_rate.max(1000.0);
        self.threshold_db = threshold_db.clamp(-60.0, 0.0);
        self.ratio = ratio.clamp(1.0, 4.0);
        self.makeup_db = makeup_db.clamp(0.0, 24.0);
        self.attack_ms = attack_ms.clamp(1.0, 200.0);
        self.release_ms = release_ms.clamp(10.0, 2000.0);
        self.update_coeffs();
    }

    fn update_coeffs(&mut self) {
        self.ga = (-1.0 / (self.attack_ms as f64 * 0.001 * self.sample_rate)).exp();
        self.gr = (-1.0 / (self.release_ms as f64 * 0.001 * self.sample_rate)).exp();
    }

    pub fn is_bypassed(&self) -> bool {
        self.threshold_db >= -0.001 && self.ratio <= 1.001 && self.makeup_db <= 0.001
    }

    pub fn process_stereo_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.is_bypassed() {
            return (left, right);
        }

        // Peak detector (max abs of stereo channels)
        let peak = left.abs().max(right.abs()).max(1e-6);
        let peak_db = 20.0 * (peak as f64).log10();

        // Ballistics (Attack / Release smoothing in log domain)
        if peak_db > self.envelope_db {
            self.envelope_db = self.ga * self.envelope_db + (1.0 - self.ga) * peak_db;
        } else {
            self.envelope_db = self.gr * self.envelope_db + (1.0 - self.gr) * peak_db;
        }

        // Static Characteristic with Soft Knee
        let thresh = self.threshold_db as f64;
        let ratio = self.ratio as f64;
        let half_knee = (self.knee_db as f64) * 0.5;

        let gain_reduction_db = if self.envelope_db <= thresh - half_knee {
            0.0
        } else if self.envelope_db >= thresh + half_knee {
            (thresh - self.envelope_db) * (1.0 - 1.0 / ratio)
        } else {
            // Within soft knee transition
            let diff = self.envelope_db - thresh + half_knee;
            -((1.0 - 1.0 / ratio) * diff * diff) / (2.0 * self.knee_db as f64)
        };

        // Total linear gain including makeup
        let total_gain_db = gain_reduction_db + self.makeup_db as f64;
        let linear_gain = 10.0f64.powf(total_gain_db / 20.0) as f32;

        (left * linear_gain, right * linear_gain)
    }

    pub fn reset(&mut self) {
        self.envelope_db = -120.0;
    }
}
