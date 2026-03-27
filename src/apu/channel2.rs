//! Channel 2 — Pulse without frequency sweep.
//!
//! Identical to CH1 in duty cycle, length counter, and envelope behaviour,
//! but has no sweep unit. Register NR20 (0xFF15) is unused/open-bus.

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

pub struct Channel2 {
    pub nr21: u8, // Duty/Len
    pub nr22: u8, // Envelope
    pub nr23: u8, // Freq LSB (write-only)
    pub nr24: u8, // Control + Freq MSB

    pub enabled: bool,
    pub dac_enabled: bool,

    pub freq_timer: u32,
    pub duty_pos: u8,

    pub length_counter: u8,

    pub env_volume: u8,
    env_timer: u8,
    env_running: bool,
}

impl Channel2 {
    pub fn new() -> Self {
        Channel2 {
            nr21: 0x3F,
            nr22: 0x00,
            nr23: 0xFF,
            nr24: 0xBF,
            enabled: false,
            dac_enabled: false,
            freq_timer: 0,
            duty_pos: 0,
            length_counter: 0,
            env_volume: 0,
            env_timer: 0,
            env_running: false,
        }
    }

    // ── Register access ──────────────────────────────────────────────────────

    pub fn read_nr21(&self) -> u8 { self.nr21 | 0x3F }
    pub fn read_nr22(&self) -> u8 { self.nr22 }
    pub fn read_nr23(&self) -> u8 { 0xFF }
    pub fn read_nr24(&self) -> u8 { self.nr24 | 0xBF }

    pub fn write_nr21(&mut self, val: u8) {
        self.nr21 = val;
        self.length_counter = 64 - (val & 0x3F);
    }

    pub fn write_nr22(&mut self, val: u8) {
        self.nr22 = val;
        self.dac_enabled = (val & 0xF8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub fn write_nr23(&mut self, val: u8) {
        self.nr23 = val;
    }

    pub fn write_nr24(&mut self, val: u8, frame_seq_step: u8) {
        let old_len_en = (self.nr24 & 0x40) != 0;
        let new_len_en = (val & 0x40) != 0;
        let trigger    = (val & 0x80) != 0;

        self.nr24 = val & 0x7F;

        if !old_len_en && new_len_en && (frame_seq_step & 1) == 1 {
            self.clock_length_inner();
        }

        if trigger {
            self.trigger(frame_seq_step);
        }
    }

    // ── Frame-sequencer clocks ───────────────────────────────────────────────

    pub fn clock_length(&mut self) {
        if (self.nr24 & 0x40) != 0 {
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
            let period = self.nr22 & 0x07;
            self.env_timer = if period == 0 { 8 } else { period };
            let add = (self.nr22 & 0x08) != 0;
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
                let freq = self.frequency();
                self.freq_timer = (2048 - freq as u32) * 4;
                self.duty_pos = (self.duty_pos + 1) & 7;
            }
        }
        DUTY_TABLE[self.duty() as usize][self.duty_pos as usize] * self.env_volume
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    pub fn frequency(&self) -> u16 {
        ((self.nr24 as u16 & 0x07) << 8) | self.nr23 as u16
    }

    pub fn freq_hz(&self) -> f32 {
        131_072.0 / (2048.0 - self.frequency() as f32)
    }

    pub fn duty(&self) -> u8 {
        (self.nr21 >> 6) & 0x03
    }

    fn trigger(&mut self, frame_seq_step: u8) {
        self.enabled = self.dac_enabled;
        if self.length_counter == 0 {
            self.length_counter = 64;
            if (frame_seq_step & 1) == 1 && (self.nr24 & 0x40) != 0 {
                self.length_counter = 63;
            }
        }
        let freq = self.frequency();
        self.freq_timer = (2048 - freq as u32) * 4;
        self.env_volume = (self.nr22 >> 4) & 0x0F;
        let period = self.nr22 & 0x07;
        self.env_timer = if period == 0 { 8 } else { period };
        self.env_running = true;
    }

    pub fn power_off(&mut self) {
        self.nr21 = 0; self.nr22 = 0; self.nr23 = 0; self.nr24 = 0;
        self.enabled = false; self.dac_enabled = false;
        self.freq_timer = 0; self.duty_pos = 0; self.length_counter = 0;
        self.env_volume = 0; self.env_timer = 0; self.env_running = false;
    }
}
