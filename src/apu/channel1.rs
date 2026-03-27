//! Channel 1 — Pulse with frequency sweep.
//!
//! CH1 is a square-wave generator with a configurable duty cycle, length
//! counter, volume envelope, and a hardware frequency sweep unit. It's the
//! only channel with sweep; CH2 is identical but omits it.

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

pub struct Channel1 {
    // Registers (stored for read-back; write-only bits masked on read)
    pub nr10: u8, // Sweep:    --PPP-NNN  (period, negate, shift)
    pub nr11: u8, // Duty/Len: DDLLllll   (duty, length — length is write-only)
    pub nr12: u8, // Envelope: VVVVDppp   (volume, dir, period)
    pub nr13: u8, // Freq LSB: write-only
    pub nr14: u8, // Control:  T-LL-FFF   (trigger, len-enable, freq MSB)

    // Status
    pub enabled: bool,
    pub dac_enabled: bool,

    // Frequency timer: counts down T-cycles, reloads at 0, advances duty position
    pub freq_timer: u32,
    pub duty_pos: u8,

    // Length counter: when it reaches 0 the channel goes silent
    pub length_counter: u8,

    // Volume envelope
    pub env_volume: u8,
    env_timer: u8,
    env_running: bool,

    // Frequency sweep
    pub sweep_timer: u8,
    pub sweep_enabled: bool,
    pub shadow_freq: u16,
    sweep_negate_used: bool, // track negate usage for obscure behaviour
}

impl Default for Channel1 {
    fn default() -> Self { Self::new() }
}

impl Channel1 {
    pub fn new() -> Self {
        Channel1 {
            nr10: 0x80,
            nr11: 0xBF,
            nr12: 0xF3,
            nr13: 0xFF,
            nr14: 0xBF,
            enabled: false,
            dac_enabled: true,
            freq_timer: 0,
            duty_pos: 0,
            length_counter: 63,
            env_volume: 0xF,
            env_timer: 3,
            env_running: true,
            sweep_timer: 0,
            sweep_enabled: false,
            shadow_freq: 0,
            sweep_negate_used: false,
        }
    }

    // ── Register access ──────────────────────────────────────────────────────

    pub fn read_nr10(&self) -> u8 { self.nr10 | 0x80 }
    pub fn read_nr11(&self) -> u8 { self.nr11 | 0x3F } // length bits write-only
    pub fn read_nr12(&self) -> u8 { self.nr12 }
    pub fn read_nr13(&self) -> u8 { 0xFF }              // freq LSB write-only
    pub fn read_nr14(&self) -> u8 { self.nr14 | 0xBF } // trigger + freq MSB write-only

    pub fn write_nr10(&mut self, val: u8) {
        // If negate was used in sweep calc and direction changes, disable channel
        let old_negate = (self.nr10 & 0x08) != 0;
        let new_negate = (val & 0x08) != 0;
        if self.sweep_negate_used && old_negate && !new_negate {
            self.enabled = false;
        }
        self.nr10 = val & 0x7F;
    }

    pub fn write_nr11(&mut self, val: u8) {
        self.nr11 = val;
        self.length_counter = 64 - (val & 0x3F);
    }

    pub fn write_nr12(&mut self, val: u8) {
        self.nr12 = val;
        self.dac_enabled = (val & 0xF8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub fn write_nr13(&mut self, val: u8) {
        self.nr13 = val;
    }

    /// `frame_seq_step` is passed so we can handle the length-counter
    /// extra-clock edge case on trigger while the sequencer is on an odd step.
    pub fn write_nr14(&mut self, val: u8, frame_seq_step: u8) {
        let old_len_en = (self.nr14 & 0x40) != 0;
        let new_len_en = (val & 0x40) != 0;
        let trigger    = (val & 0x80) != 0;

        self.nr14 = val & 0x7F;

        // Extra length clock: enabling length counter on an odd sequencer step
        // clocks the length counter immediately.
        if !old_len_en && new_len_en && (frame_seq_step & 1) == 1 {
            self.clock_length_inner();
        }

        if trigger {
            self.trigger(frame_seq_step);
        }
    }

    // ── Frame-sequencer clocks ───────────────────────────────────────────────

    /// Steps 0, 2, 4, 6 — clock the length counter.
    pub fn clock_length(&mut self) {
        if (self.nr14 & 0x40) != 0 {
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

    /// Steps 2 and 6 — clock the frequency sweep unit.
    pub fn clock_sweep(&mut self) {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            let period = (self.nr10 >> 4) & 0x07;
            self.sweep_timer = if period == 0 { 8 } else { period };
            if self.sweep_enabled && period != 0 {
                let new_freq = self.calc_new_freq();
                let shift = self.nr10 & 0x07;
                if new_freq <= 2047 && shift != 0 {
                    self.shadow_freq = new_freq;
                    self.nr13 = (new_freq & 0xFF) as u8;
                    self.nr14 = (self.nr14 & 0xF8) | ((new_freq >> 8) as u8 & 0x07);
                    // Spec requires a second overflow check after writing
                    self.calc_new_freq();
                }
            }
        }
    }

    /// Step 7 — clock the volume envelope.
    pub fn clock_envelope(&mut self) {
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 && self.env_running {
            let period = self.nr12 & 0x07;
            self.env_timer = if period == 0 { 8 } else { period };
            let add = (self.nr12 & 0x08) != 0;
            if add && self.env_volume < 15 {
                self.env_volume += 1;
            } else if !add && self.env_volume > 0 {
                self.env_volume -= 1;
            } else {
                self.env_running = false;
            }
        }
    }

    // ── Tick (per-instruction) ───────────────────────────────────────────────

    /// Advance the frequency timer by `cycles` T-cycles.
    /// Returns the current output amplitude (0–15).
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
                let freq = self.frequency();
                self.freq_timer = (2048 - freq as u32) * 4;
                self.duty_pos = (self.duty_pos + 1) & 7;
            }
        }
        DUTY_TABLE[self.duty() as usize][self.duty_pos as usize] * self.env_volume
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    pub fn frequency(&self) -> u16 {
        ((self.nr14 as u16 & 0x07) << 8) | self.nr13 as u16
    }

    pub fn freq_hz(&self) -> f32 {
        131_072.0 / (2048.0 - self.frequency() as f32)
    }

    pub fn duty(&self) -> u8 {
        (self.nr11 >> 6) & 0x03
    }

    fn calc_new_freq(&mut self) -> u16 {
        let negate = (self.nr10 & 0x08) != 0;
        let shift = (self.nr10 & 0x07) as u16;
        let delta = self.shadow_freq >> shift;
        let new_freq = if negate {
            self.sweep_negate_used = true;
            self.shadow_freq.wrapping_sub(delta)
        } else {
            self.shadow_freq.wrapping_add(delta)
        };
        if new_freq > 2047 {
            self.enabled = false;
        }
        new_freq
    }

    fn trigger(&mut self, frame_seq_step: u8) {
        self.enabled = self.dac_enabled;
        if self.length_counter == 0 {
            self.length_counter = 64;
            // If re-triggered on an odd step with length enabled, clock immediately
            if (frame_seq_step & 1) == 1 && (self.nr14 & 0x40) != 0 {
                self.length_counter = 63;
            }
        }
        let freq = self.frequency();
        self.freq_timer = (2048 - freq as u32) * 4;

        // Envelope reload
        self.env_volume = (self.nr12 >> 4) & 0x0F;
        let period = self.nr12 & 0x07;
        self.env_timer = if period == 0 { 8 } else { period };
        self.env_running = true;

        // Sweep reload
        self.shadow_freq = freq;
        self.sweep_negate_used = false;
        let sweep_period = (self.nr10 >> 4) & 0x07;
        let sweep_shift = self.nr10 & 0x07;
        self.sweep_timer = if sweep_period == 0 { 8 } else { sweep_period };
        self.sweep_enabled = sweep_period != 0 || sweep_shift != 0;
        // Initial overflow check (doesn't write shadow freq)
        if sweep_shift != 0 {
            self.calc_new_freq();
        }
    }

    /// Called when APU power is switched off — clears all registers.
    pub fn power_off(&mut self) {
        self.nr10 = 0; self.nr11 = 0; self.nr12 = 0; self.nr13 = 0; self.nr14 = 0;
        self.enabled = false; self.dac_enabled = false;
        self.freq_timer = 0; self.duty_pos = 0; self.length_counter = 0;
        self.env_volume = 0; self.env_timer = 0; self.env_running = false;
        self.sweep_timer = 0; self.sweep_enabled = false; self.shadow_freq = 0;
        self.sweep_negate_used = false;
    }
}
