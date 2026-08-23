#include "../signalsmith-stretch.h"

// Wrapper to interface raw float** pointers with Signalsmith's template expectations
struct ChannelBuffers {
    const float* const* buffers;
    const float* operator[](int c) const {
        return buffers[c];
    }
};

struct MutableChannelBuffers {
    float* const* buffers;
    float* operator[](int c) const {
        return buffers[c];
    }
};

extern "C" {

void* signalsmith_stretch_create(int channels, float sample_rate) {
    auto* stretch = new signalsmith::stretch::SignalsmithStretch<float>();
    stretch->presetDefault(channels, sample_rate);
    return static_cast<void*>(stretch);
}

void signalsmith_stretch_destroy(void* instance) {
    if (instance) {
        delete static_cast<signalsmith::stretch::SignalsmithStretch<float>*>(instance);
    }
}

void signalsmith_stretch_set_transpose_factor(void* instance, float factor) {
    if (instance) {
        auto* stretch = static_cast<signalsmith::stretch::SignalsmithStretch<float>*>(instance);
        stretch->setTransposeFactor(factor);
    }
}

void signalsmith_stretch_set_transpose_semitones(void* instance, float semitones) {
    if (instance) {
        auto* stretch = static_cast<signalsmith::stretch::SignalsmithStretch<float>*>(instance);
        stretch->setTransposeSemitones(semitones);
    }
}

void signalsmith_stretch_process(void* instance, const float* const* input, int input_samples, float* const* output, int output_samples) {
    if (instance) {
        auto* stretch = static_cast<signalsmith::stretch::SignalsmithStretch<float>*>(instance);
        ChannelBuffers in_bufs{input};
        MutableChannelBuffers out_bufs{output};
        stretch->process(in_bufs, input_samples, out_bufs, output_samples);
    }
}

void signalsmith_stretch_reset(void* instance) {
    if (instance) {
        auto* stretch = static_cast<signalsmith::stretch::SignalsmithStretch<float>*>(instance);
        stretch->reset();
    }
}

} // extern "C"
