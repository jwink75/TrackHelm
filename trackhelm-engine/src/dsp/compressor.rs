// High-precision Feedforward Dynamic Compressor with Multiple Types & Dual-Stage Processing

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CompType {
    Vintage,
    Modern,
    FET,
    Opto,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CompRouting {
    Series,
    Parallel,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompStageParams {
    pub enabled: bool,
    pub comp_type: CompType,
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
}

impl Default for CompStageParams {
    fn default() -> Self {
        Self {
            enabled: true,
            comp_type: CompType::Vintage,
            threshold_db: 0.0,
            ratio: 1.0,
            knee_db: 3.0,
            attack_ms: 30.0,
            release_ms: 300.0,
            makeup_db: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SingleCompressor {
    sample_rate: f64,
    pub params: CompStageParams,
    envelope_db: f64,
    ga: f64,
    gr: f64,
    pub last_gr_db: f32,
}

impl SingleCompressor {
    pub fn new(sample_rate: f64, params: CompStageParams) -> Self {
        let mut comp = Self {
            sample_rate: sample_rate.max(1000.0),
            params,
            envelope_db: -120.0,
            ga: 0.0,
            gr: 0.0,
            last_gr_db: 0.0,
        };
        comp.update_coeffs();
        comp
    }

    pub fn set_params(&mut self, sample_rate: f64, params: CompStageParams) {
        self.sample_rate = sample_rate.max(1000.0);
        self.params = params;
        self.update_coeffs();
    }

    pub fn update_coeffs(&mut self) {
        let mut effective_attack = self.params.attack_ms;
        let mut effective_release = self.params.release_ms;

        match self.params.comp_type {
            CompType::FET => {
                // Ultra fast FET attack (0.02ms - 0.8ms scaling)
                effective_attack = (self.params.attack_ms * 0.1).clamp(0.05, 50.0);
            }
            CompType::Opto => {
                // Smooth optical response
                effective_attack = self.params.attack_ms.clamp(10.0, 100.0);
                effective_release = self.params.release_ms.clamp(100.0, 2500.0);
            }
            _ => {}
        }

        self.ga = (-1.0 / (effective_attack as f64 * 0.001 * self.sample_rate)).exp();
        self.gr = (-1.0 / (effective_release as f64 * 0.001 * self.sample_rate)).exp();
    }

    pub fn is_bypassed(&self) -> bool {
        !self.params.enabled
            || (self.params.threshold_db >= -0.001
                && self.params.ratio <= 1.001
                && self.params.makeup_db.abs() <= 0.001)
    }

    pub fn process_stereo_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.is_bypassed() {
            self.last_gr_db = 0.0;
            return (left, right);
        }

        let peak = left.abs().max(right.abs()).max(1e-6);
        let peak_db = 20.0 * (peak as f64).log10();

        if peak_db > self.envelope_db {
            self.envelope_db = self.ga * self.envelope_db + (1.0 - self.ga) * peak_db;
        } else {
            self.envelope_db = self.gr * self.envelope_db + (1.0 - self.gr) * peak_db;
        }

        let thresh = self.params.threshold_db as f64;
        let ratio = self.params.ratio.max(1.0) as f64;
        let knee = self.params.knee_db.max(0.1) as f64;
        let half_knee = knee * 0.5;

        let gain_reduction_db = if self.envelope_db <= thresh - half_knee {
            0.0
        } else if self.envelope_db >= thresh + half_knee {
            (thresh - self.envelope_db) * (1.0 - 1.0 / ratio)
        } else {
            let diff = self.envelope_db - thresh + half_knee;
            -((1.0 - 1.0 / ratio) * diff * diff) / (2.0 * knee)
        };

        self.last_gr_db = gain_reduction_db as f32;

        let total_gain_db = gain_reduction_db + self.params.makeup_db as f64;
        let linear_gain = 10.0f64.powf(total_gain_db / 20.0) as f32;

        (left * linear_gain, right * linear_gain)
    }

    pub fn reset(&mut self) {
        self.envelope_db = -120.0;
        self.last_gr_db = 0.0;
    }
}

#[derive(Clone, Debug)]
pub struct DualCompressor {
    pub stage1: SingleCompressor,
    pub stage2: SingleCompressor,
    pub routing: CompRouting,
    pub parallel_blend: f32, // 0.0 = 100% Stage 1, 1.0 = 100% Stage 2, 0.5 = 50/50 blend
}

impl DualCompressor {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            stage1: SingleCompressor::new(sample_rate, CompStageParams::default()),
            stage2: SingleCompressor::new(
                sample_rate,
                CompStageParams {
                    enabled: false,
                    comp_type: CompType::Opto,
                    threshold_db: 0.0,
                    ratio: 1.0,
                    knee_db: 3.0,
                    attack_ms: 50.0,
                    release_ms: 500.0,
                    makeup_db: 0.0,
                },
            ),
            routing: CompRouting::Series,
            parallel_blend: 0.5,
        }
    }

    pub fn is_bypassed(&self) -> bool {
        self.stage1.is_bypassed() && self.stage2.is_bypassed()
    }

    pub fn process_stereo_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.is_bypassed() {
            return (left, right);
        }

        match self.routing {
            CompRouting::Series => {
                let (l1, r1) = self.stage1.process_stereo_frame(left, right);
                self.stage2.process_stereo_frame(l1, r1)
            }
            CompRouting::Parallel => {
                let (l1, r1) = self.stage1.process_stereo_frame(left, right);
                let (l2, r2) = self.stage2.process_stereo_frame(left, right);
                let w2 = self.parallel_blend.clamp(0.0, 1.0);
                let w1 = 1.0 - w2;
                (l1 * w1 + l2 * w2, r1 * w1 + r2 * w2)
            }
        }
    }

    pub fn reset(&mut self) {
        self.stage1.reset();
        self.stage2.reset();
    }
}
