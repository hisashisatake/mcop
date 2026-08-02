// Minimal extern "C" shim around ymfm OPZ (YM2414).
// Rust writes a register sequence, then generates PCM.
#include "ymfm_opz.h"
#include <cstdint>

using namespace ymfm;

namespace {
struct Renderer {
    ymfm_interface intf; // no pure virtuals -> default impl is fine
    ym2414 chip;
    Renderer() : chip(intf) { chip.reset(); }
};
} // namespace

extern "C" {

void *opzref_create() { return new Renderer(); }

void opzref_destroy(void *h) { delete static_cast<Renderer *>(h); }

// Native output sample rate for the given input clock (Hz).
unsigned opzref_sample_rate(void *h, unsigned clock) {
    return static_cast<Renderer *>(h)->chip.sample_rate(clock);
}

// Write one register: address port then data port.
void opzref_write_reg(void *h, unsigned reg, unsigned char data) {
    auto *r = static_cast<Renderer *>(h);
    r->chip.write(0, static_cast<uint8_t>(reg & 0xff)); // address port
    r->chip.write(1, data);                             // data port
}

// Generate n mono float samples (avg of L/R, roughly [-1,1]) into out.
void opzref_generate(void *h, float *out, unsigned n) {
    auto *r = static_cast<Renderer *>(h);
    ym2414::output_data od;
    for (unsigned i = 0; i < n; i++) {
        r->chip.generate(&od, 1);
        out[i] = static_cast<float>(od.data[0] + od.data[1]) * (0.5f / 32768.0f);
    }
}

} // extern "C"
