//! Channel 4 — Noise.
//!
//! Generates pseudo-random noise via a 15-bit (or 7-bit) Linear Feedback Shift
//! Register (LFSR). The clock rate and LFSR width are controlled by NR43.
//!
//! Clock frequency = 524288 / r / 2^(s+1)  Hz
//!   where r = NR43 bits 2-0 (divider, 0 → use 0.5)
//!         s = NR43 bits 7-4 (shift)
//!
//! Timer period in T-cycles = CPU_CLOCK / freq = 8 * r * 2^(s+1)
//!   (for r=0 use 0.5 → period = 4 * 2^(s+1) = 2^(s+3))

pub struct Channel4 {
    pub nr41: u8, // Length (bits 5-0)
    pub nr42: u8, // Envelope
    pub nr43: u8, // Clock config: ssss_wrrr
    pub nr44: u8, // Control: T---LL--

    pub enabled: bool,
    pub dac_enabled: bool,

    /// LFSR: 15-bit register; bit 0 is the output bit.
    pub lfsr: u16,
    pub freq_timer: u32,

    pub length_counter: u8,

    pub env_volume: u8,
    env_timer: u8,
    env_running: bool,
}

impl Default for Channel4 {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel4 {
    pub fn new() -> Self {
        Channel4 {
            nr41: 0xFF,
            nr42: 0x00,
            nr43: 0x00,
            nr44: 0xBF,
            enabled: false,
            dac_enabled: false,
            lfsr: 0x7FFF,
            freq_timer: 0,
            length_counter: 0,
            env_volume: 0,
            env_timer: 0,
            env_running: false,
        }
    }

    // ── Register access ──────────────────────────────────────────────────────

    pub fn read_nr41(&self) -> u8 {
        0xFF
    }
    pub fn read_nr42(&self) -> u8 {
        self.nr42
    }
    pub fn read_nr43(&self) -> u8 {
        self.nr43
    }
    pub fn read_nr44(&self) -> u8 {
        self.nr44 | 0xBF
    }

    pub fn write_nr41(&mut self, val: u8) {
        self.nr41 = val;
        self.length_counter = 64 - (val & 0x3F);
    }

    pub fn write_nr42(&mut self, val: u8) {
        self.nr42 = val;
        self.dac_enabled = (val & 0xF8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub fn write_nr43(&mut self, val: u8) {
        self.nr43 = val;
    }

    pub fn write_nr44(&mut self, val: u8, frame_seq_step: u8) {
        let old_len_en = (self.nr44 & 0x40) != 0;
        let new_len_en = (val & 0x40) != 0;
        let trigger = (val & 0x80) != 0;

        self.nr44 = val & 0x7F;

        if !old_len_en && new_len_en && (frame_seq_step & 1) == 1 {
            self.clock_length_inner();
        }

        if trigger {
            self.trigger(frame_seq_step);
        }
    }

    // ── Frame-sequencer clocks ───────────────────────────────────────────────

    pub fn clock_length(&mut self) {
        if (self.nr44 & 0x40) != 0 {
            self.clock_length_inner();
        }
    }

    fn clock_length_inner(&mut self) {
        if self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled = false;
            }
        }
    }

    pub fn clock_envelope(&mut self) {
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 && self.env_running {
            let period = self.nr42 & 0x07;
            self.env_timer = if period == 0 { 8 } else { period };
            let add = (self.nr42 & 0x08) != 0;
            if add && self.env_volume < 15 {
                self.env_volume += 1;
            } else if !add && self.env_volume > 0 {
                self.env_volume -= 1;
            } else {
                self.env_running = false;
            }
        }
    }

    // ── Tick ─────────────────────────────────────────────────────────────────

    pub fn tick(&mut self, cycles: u32) -> u8 {
        if !self.enabled || !self.dac_enabled {
            return 0;
        }
        let mut remaining = cycles;
        while remaining > 0 {
            let step = remaining.min(self.freq_timer);
            self.freq_timer -= step;
            remaining -= step;
            if self.freq_timer == 0 {
                self.freq_timer = self.period();
                self.clock_lfsr();
            }
        }
        // Output is 1 when LFSR bit 0 is CLEAR (inverted)
        if (self.lfsr & 1) == 0 {
            self.env_volume
        } else {
            0
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// True if 7-bit LFSR mode (NR43 bit 3 set).
    pub fn is_short_lfsr(&self) -> bool {
        (self.nr43 & 0x08) != 0
    }

    pub fn clock_shift(&self) -> u8 {
        (self.nr43 >> 4) & 0x0F
    }

    pub fn clock_divider(&self) -> u8 {
        self.nr43 & 0x07
    }

    /// Approximate output frequency in Hz.
    pub fn freq_hz(&self) -> f32 {
        let r = self.clock_divider();
        let s = self.clock_shift();
        let r_eff = if r == 0 { 0.5_f32 } else { r as f32 };
        524_288.0 / r_eff / (1 << (s + 1)) as f32
    }

    fn period(&self) -> u32 {
        let r = self.clock_divider() as u32;
        let s = self.clock_shift() as u32;
        if r == 0 {
            // r=0 treated as r=0.5 → period = 4 * 2^(s+1)
            4u32 << (s + 1)
        } else {
            8 * r * (1u32 << (s + 1))
        }
    }

    fn clock_lfsr(&mut self) {
        let xor_bit = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
        self.lfsr = (self.lfsr >> 1) | (xor_bit << 14);
        if self.is_short_lfsr() {
            self.lfsr = (self.lfsr & !(1 << 6)) | (xor_bit << 6);
        }
    }

    fn trigger(&mut self, frame_seq_step: u8) {
        self.enabled = self.dac_enabled;
        if self.length_counter == 0 {
            self.length_counter = 64;
            if (frame_seq_step & 1) == 1 && (self.nr44 & 0x40) != 0 {
                self.length_counter = 63;
            }
        }
        self.freq_timer = self.period();
        self.lfsr = 0x7FFF;
        self.env_volume = (self.nr42 >> 4) & 0x0F;
        let period = self.nr42 & 0x07;
        self.env_timer = if period == 0 { 8 } else { period };
        self.env_running = true;
    }

    pub fn power_off(&mut self) {
        self.nr41 = 0;
        self.nr42 = 0;
        self.nr43 = 0;
        self.nr44 = 0;
        self.enabled = false;
        self.dac_enabled = false;
        self.lfsr = 0x7FFF;
        self.freq_timer = 0;
        self.length_counter = 0;
        self.env_volume = 0;
        self.env_timer = 0;
        self.env_running = false;
    }
}
