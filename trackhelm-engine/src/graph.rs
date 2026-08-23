pub trait AudioNode {
    fn process(&mut self, output: &mut [&mut [f32]]);
}

pub struct AudioGraph {
    _nodes: Vec<Box<dyn AudioNode + Send>>,
}

impl AudioGraph {
    pub fn new() -> Self {
        Self { _nodes: Vec::new() }
    }

    pub fn render(&mut self, buffer: &mut [&mut [f32]]) {
        // Core mix render loop placeholder
        for channel in buffer.iter_mut() {
            for sample in channel.iter_mut() {
                *sample = 0.0;
            }
        }
    }
}
