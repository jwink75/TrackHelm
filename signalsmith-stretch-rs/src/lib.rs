use std::ffi::c_void;

extern "C" {
    fn signalsmith_stretch_create(channels: i32, sample_rate: f32) -> *mut c_void;
    fn signalsmith_stretch_destroy(instance: *mut c_void);
    fn signalsmith_stretch_set_transpose_factor(instance: *mut c_void, factor: f32);
    fn signalsmith_stretch_set_transpose_semitones(instance: *mut c_void, semitones: f32);
    fn signalsmith_stretch_process(
        instance: *mut c_void,
        input: *const *const f32,
        input_samples: i32,
        output: *const *mut f32,
        output_samples: i32,
    );
    fn signalsmith_stretch_reset(instance: *mut c_void);
}

pub struct SignalsmithStretch {
    instance: *mut c_void,
    channels: usize,
}

// The internal pointer is safe to send across threads and use.
// We'll manage thread safety using standard Rust synchronization tools if required,
// but typically the engine will own this structure on the real-time audio thread.
unsafe impl Send for SignalsmithStretch {}
unsafe impl Sync for SignalsmithStretch {}

impl SignalsmithStretch {
    pub fn new(channels: usize, sample_rate: f32) -> Self {
        let instance = unsafe { signalsmith_stretch_create(channels as i32, sample_rate) };
        assert!(!instance.is_null(), "Failed to create SignalsmithStretch instance");
        Self { instance, channels }
    }

    pub fn set_transpose_factor(&self, factor: f32) {
        unsafe { signalsmith_stretch_set_transpose_factor(self.instance, factor) };
    }

    pub fn set_transpose_semitones(&self, semitones: f32) {
        unsafe { signalsmith_stretch_set_transpose_semitones(self.instance, semitones) };
    }

    pub fn process(&self, input: &[&[f32]], output: &mut [&mut [f32]]) {
        assert_eq!(input.len(), self.channels);
        assert_eq!(output.len(), self.channels);

        let input_samples = input[0].len();
        let output_samples = output[0].len();

        for i in 1..self.channels {
            assert_eq!(input[i].len(), input_samples, "Mismatched input channel lengths");
            assert_eq!(output[i].len(), output_samples, "Mismatched output channel lengths");
        }

        // Convert nested slices to pointers of pointers
        let input_ptrs: Vec<*const f32> = input.iter().map(|ch| ch.as_ptr()).collect();
        let mut output_ptrs: Vec<*mut f32> = output.iter_mut().map(|ch| ch.as_mut_ptr()).collect();

        unsafe {
            signalsmith_stretch_process(
                self.instance,
                input_ptrs.as_ptr(),
                input_samples as i32,
                output_ptrs.as_mut_ptr(),
                output_samples as i32,
            );
        }
    }

    pub fn reset(&self) {
        unsafe { signalsmith_stretch_reset(self.instance) };
    }
}

impl Drop for SignalsmithStretch {
    fn drop(&mut self) {
        unsafe { signalsmith_stretch_destroy(self.instance) };
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_destroy() {
        let stretch = SignalsmithStretch::new(2, 44100.0);
        stretch.set_transpose_semitones(2.0);
        stretch.reset();
    }
}
