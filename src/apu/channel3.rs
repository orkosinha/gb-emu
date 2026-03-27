//! Channel 3 — Wave output.
//!
//! Plays a 32-nibble (4-bit sample) waveform stored in wave RAM (0xFF30–0xFF3F).
//! Volume is controlled by a 2-bit shift: mute, 100%, 50%, or 25%.
//! The frequency timer runs at 2T per tick rather than 4T (twice as fast as pulse).

pub struct Channel3 {
    pub nr30: u8, // DAC enable (bit 7)
    pub nr31: u8, // Length timer
    pub nr32: u8, // Volume code (bits 6-5): 00=mute, 01=100%, 10=50%, 11=25%
    pub nr33: u8, // Freq LSB (write-only)
    pub nr34: u8, // Control + Freq MSB

    pub enabled: bool,
    pub dac_enabled: bool,

    pub freq_timer: u32,
    pub wave_pos: u8,     // 0–31 into wave RAM nibbles
    sample_buffer: u8,    // latched nibble (resampled once per timer tick)

    pub length_counter: u16, // 256 counts for CH3 (not 64)

    /// Wave RAM: 16 bytes = 32 4-bit samples.
    pub wave_ram: [u8; 16],
}

impl Default for Channel3 {
    fn default() -> Self { Self::new() }
}

impl Channel3 {
    pub fn new() -> Self {
        Channel3 {
            nr30: 0x7F,
            nr31: 0xFF,
            nr32: 0x9F,
            nr33: 0xFF,
            nr34: 0xBF,
            enabled: false,
            dac_enabled: false,
            freq_timer: 0,
            wave_pos: 0,
            sample_buffer: 0,
            length_counter: 0,
            // Wave RAM power-on content varies by revision; zeros is safe
            wave_ram: [0u8; 16],
        }
    }

    // ── Register access ──────────────────────────────────────────────────────

    pub fn read_nr30(&self) -> u8 { self.nr30 | 0x7F }
    pub fn read_nr31(&self) -> u8 { 0xFF }           // length write-only
    pub fn read_nr32(&self) -> u8 { self.nr32 | 0x9F }
    pub fn read_nr33(&self) -> u8 { 0xFF }           // freq LSB write-only
    pub fn read_nr34(&self) -> u8 { self.nr34 | 0xBF }

    /// Wave RAM reads: when channel is active the last latched byte is returned
    /// (hardware read-back quirk); for simplicity we return the raw byte.
    pub fn read_wave_ram(&self, offset: u8) -> u8 {
        self.wave_ram[(offset & 0x0F) as usize]
    }

    pub fn write_wave_ram(&mut self, offset: u8, val: u8) {
        self.wave_ram[(offset & 0x0F) as usize] = val;
    }

    pub fn write_nr30(&mut self, val: u8) {
        self.nr30 = val;
        self.dac_enabled = (val & 0x80) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub fn write_nr31(&mut self, val: u8) {
        self.nr31 = val;
        self.length_counter = 256 - val as u16;
    }

    pub fn write_nr32(&mut self, val: u8) {
        self.nr32 = val;
    }

    pub fn write_nr33(&mut self, val: u8) {
        self.nr33 = val;
    }

    pub fn write_nr34(&mut self, val: u8, frame_seq_step: u8) {
        let old_len_en = (self.nr34 & 0x40) != 0;
        let new_len_en = (val & 0x40) != 0;
        let trigger    = (val & 0x80) != 0;

        self.nr34 = val & 0x7F;

        if !old_len_en && new_len_en && (frame_seq_step & 1) == 1 {
            self.clock_length_inner();
        }

        if trigger {
            self.trigger(frame_seq_step);
        }
    }

    // ── Frame-sequencer clocks ───────────────────────────────────────────────

    pub fn clock_length(&mut self) {
        if (self.nr34 & 0x40) != 0 {
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

    // ── Tick ─────────────────────────────────────────────────────────────────

    /// Advance the wave player by `cycles` T-cycles.
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
                self.freq_timer = (2048 - freq as u32) * 2; // wave runs at 2T
                self.wave_pos = (self.wave_pos + 1) & 31;
                let byte = self.wave_ram[(self.wave_pos / 2) as usize];
                // High nibble at even positions, low nibble at odd
                self.sample_buffer = if self.wave_pos & 1 == 0 {
                    (byte >> 4) & 0x0F
                } else {
                    byte & 0x0F
                };
            }
        }
        // Apply volume shift
        let shift = match (self.nr32 >> 5) & 0x03 {
            0 => return 0,  // mute
            1 => 0,         // 100% (no shift)
            2 => 1,         // 50%
            3 => 2,         // 25%
            _ => unreachable!(),
        };
        self.sample_buffer >> shift
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    pub fn frequency(&self) -> u16 {
        ((self.nr34 as u16 & 0x07) << 8) | self.nr33 as u16
    }

    pub fn freq_hz(&self) -> f32 {
        65_536.0 / (2048.0 - self.frequency() as f32)
    }

    pub fn volume_code(&self) -> u8 {
        (self.nr32 >> 5) & 0x03
    }

    fn trigger(&mut self, frame_seq_step: u8) {
        self.enabled = self.dac_enabled;
        if self.length_counter == 0 {
            self.length_counter = 256;
            if (frame_seq_step & 1) == 1 && (self.nr34 & 0x40) != 0 {
                self.length_counter = 255;
            }
        }
        let freq = self.frequency();
        self.freq_timer = (2048 - freq as u32) * 2;
        self.wave_pos = 0;
    }

    pub fn power_off(&mut self) {
        self.nr30 = 0; self.nr31 = 0; self.nr32 = 0; self.nr33 = 0; self.nr34 = 0;
        self.enabled = false; self.dac_enabled = false;
        self.freq_timer = 0; self.wave_pos = 0; self.sample_buffer = 0;
        self.length_counter = 0;
        // Wave RAM is NOT cleared on power-off
    }
}
