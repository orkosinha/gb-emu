//! Game Boy APU — Audio Processing Unit.
//!
//! Implements the four audio channels of the original Game Boy hardware:
//!
//! - **CH1** (0xFF10–0xFF14): Pulse + frequency sweep
//! - **CH2** (0xFF16–0xFF19): Pulse (no sweep)
//! - **CH3** (0xFF1A–0xFF1E, 0xFF30–0xFF3F): Wave output
//! - **CH4** (0xFF20–0xFF23): Noise (LFSR)
//!
//! ## Frame Sequencer
//!
//! The frame sequencer runs at 512 Hz, driven by the falling edge of bit 12
//! of the timer's internal `div_counter` (= DIV register bit 4).  Each of its
//! 8 steps clocks a different combination of length, sweep, and envelope units.
//!
//! ## Sample generation
//!
//! During each `tick()` the APU accumulates fractional cycle counts. When the
//! accumulator exceeds one sample's worth of cycles it mixes the four DAC
//! outputs into a stereo f32 pair and pushes it to `sample_buf`.  The buffer
//! is drained once per frame by the FFI / WASM layers.
//!
//! Target sample rate: 44 100 Hz  →  ~95.1 T-cycles per sample.

pub mod channel1;
pub mod channel2;
pub mod channel3;
pub mod channel4;

use channel1::Channel1;
use channel2::Channel2;
use channel3::Channel3;
use channel4::Channel4;

/// Target audio sample rate (Hz).
pub const SAMPLE_RATE: u32 = 44_100;

/// CPU T-cycles per second.
const CPU_CLOCK: f64 = 4_194_304.0;

/// T-cycles per output sample (fractional).
const CYCLES_PER_SAMPLE: f64 = CPU_CLOCK / SAMPLE_RATE as f64;

// ── Debug state ─────────────────────────────────────────────────────────────

/// Per-channel debug snapshot — enough data to render a LSDJ-style tracker UI.
#[derive(Clone, Default)]
pub struct ChannelDebug {
    pub enabled: bool,
    pub dac_on: bool,
    pub volume: u8,       // 0–15
    pub freq_reg: u16,    // raw 11-bit freq register (CH1/2/3)
    pub freq_hz: f32,
    /// Duty cycle index 0–3 (CH1/2), volume code 0–3 (CH3), 0 for CH4.
    pub duty_or_vol: u8,
    /// Current position within duty/wave (0–7 for CH1/2, 0–31 for CH3).
    pub pos: u8,
    /// Envelope direction: true = increase.
    pub env_add: bool,
    /// Envelope period (0–7).
    pub env_period: u8,
    /// Length counter value.
    pub length: u16,
    /// Length counter enabled.
    pub length_enabled: bool,
    /// Sweep period (CH1 only, 0 otherwise).
    pub sweep_period: u8,
    /// Sweep shift (CH1 only).
    pub sweep_shift: u8,
    /// Sweep direction: true = subtract (CH1 only).
    pub sweep_negate: bool,
    /// 15-bit LFSR state (CH4 only).
    pub lfsr: u16,
    /// Short (7-bit) LFSR mode (CH4 only).
    pub lfsr_short: bool,
    /// MIDI note number (0–127, 255 = unknown).
    pub midi_note: u8,
}

/// Full APU debug snapshot for one frame.
#[derive(Clone, Default)]
pub struct ApuDebugState {
    pub powered: bool,
    /// NR50: master volume + VIN routing.
    pub nr50: u8,
    /// NR51: left/right channel panning mask.
    pub nr51: u8,
    /// NR52: power + channel active bits (read-back).
    pub nr52: u8,
    /// Frame sequencer step (0–7).
    pub frame_seq_step: u8,
    pub ch: [ChannelDebug; 4],
    /// Wave RAM snapshot (16 bytes = 32 nibbles).
    pub wave_ram: [u8; 16],
}

// ── APU ──────────────────────────────────────────────────────────────────────

pub struct Apu {
    pub ch1: Channel1,
    pub ch2: Channel2,
    pub ch3: Channel3,
    pub ch4: Channel4,

    /// NR50 — master volume / VIN panning.
    pub nr50: u8,
    /// NR51 — sound output terminal routing (L/R per channel).
    pub nr51: u8,
    /// NR52 — global power bit (bit 7); channel status bits set by hardware.
    pub nr52: u8,

    /// Frame sequencer step 0–7.
    frame_seq_step: u8,
    /// Previous value of div_counter bit 12 for falling-edge detection.
    prev_div_bit12: bool,

    /// Fractional cycle accumulator for sample generation.
    sample_accum: f64,

    /// Output sample buffer: interleaved (L, R) f32 pairs.
    pub sample_buf: Vec<f32>,
}

impl Default for Apu {
    fn default() -> Self { Self::new() }
}

impl Apu {
    pub fn new() -> Self {
        Apu {
            ch1: Channel1::new(),
            ch2: Channel2::new(),
            ch3: Channel3::new(),
            ch4: Channel4::new(),
            nr50: 0x77,
            nr51: 0xF3,
            nr52: 0xF1,
            frame_seq_step: 0,
            prev_div_bit12: false,
            sample_accum: 0.0,
            sample_buf: Vec::with_capacity(1024),
        }
    }

    // ── Main tick ────────────────────────────────────────────────────────────

    /// Advance the APU by `cycles` T-cycles.
    ///
    /// `div_counter` is the timer's internal 16-bit counter; bit 12 of it
    /// drives the 512 Hz frame sequencer via falling-edge detection.
    pub fn tick(&mut self, cycles: u32, div_counter: u16) {
        if !self.powered() {
            return;
        }

        // ── Frame sequencer ──────────────────────────────────────────────────
        // Detect falling edge of div_counter bit 12 (= DIV bit 4 = 512 Hz).
        // We only check the *final* value once per instruction tick; this is
        // accurate for instructions ≤ ~8190 cycles (well within GB limits).
        let bit12 = (div_counter >> 12) & 1 == 1;
        if self.prev_div_bit12 && !bit12 {
            self.step_frame_sequencer();
        }
        self.prev_div_bit12 = bit12;

        // ── Clock channels ───────────────────────────────────────────────────
        let out1 = self.ch1.tick(cycles);
        let out2 = self.ch2.tick(cycles);
        let out3 = self.ch3.tick(cycles);
        let out4 = self.ch4.tick(cycles);

        // ── Sample generation ────────────────────────────────────────────────
        self.sample_accum += cycles as f64;
        while self.sample_accum >= CYCLES_PER_SAMPLE {
            self.sample_accum -= CYCLES_PER_SAMPLE;
            self.push_sample(out1, out2, out3, out4);
        }
    }

    // ── Register read / write ────────────────────────────────────────────────

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            // CH1
            0xFF10 => self.ch1.read_nr10(),
            0xFF11 => self.ch1.read_nr11(),
            0xFF12 => self.ch1.read_nr12(),
            0xFF13 => self.ch1.read_nr13(),
            0xFF14 => self.ch1.read_nr14(),
            // CH2 (0xFF15 unused)
            0xFF15 => 0xFF,
            0xFF16 => self.ch2.read_nr21(),
            0xFF17 => self.ch2.read_nr22(),
            0xFF18 => self.ch2.read_nr23(),
            0xFF19 => self.ch2.read_nr24(),
            // CH3
            0xFF1A => self.ch3.read_nr30(),
            0xFF1B => self.ch3.read_nr31(),
            0xFF1C => self.ch3.read_nr32(),
            0xFF1D => self.ch3.read_nr33(),
            0xFF1E => self.ch3.read_nr34(),
            // CH4 (0xFF1F unused)
            0xFF1F => 0xFF,
            0xFF20 => self.ch4.read_nr41(),
            0xFF21 => self.ch4.read_nr42(),
            0xFF22 => self.ch4.read_nr43(),
            0xFF23 => self.ch4.read_nr44(),
            // Control
            0xFF24 => self.nr50,
            0xFF25 => self.nr51,
            0xFF26 => self.read_nr52(),
            // Unused 0xFF27–0xFF2F
            0xFF27..=0xFF2F => 0xFF,
            // Wave RAM
            0xFF30..=0xFF3F => self.ch3.read_wave_ram((addr - 0xFF30) as u8),
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        // When APU is powered off only NR52 and wave RAM are writable
        if !self.powered() && addr != 0xFF26 && !(0xFF30..=0xFF3F).contains(&addr) {
            // DMG allows length counters to be written while powered off
            match addr {
                0xFF11 => self.ch1.write_nr11(val & 0x3F),
                0xFF16 => self.ch2.write_nr21(val & 0x3F),
                0xFF1B => self.ch3.write_nr31(val),
                0xFF20 => self.ch4.write_nr41(val & 0x3F),
                _ => {}
            }
            return;
        }
        match addr {
            0xFF10 => self.ch1.write_nr10(val),
            0xFF11 => self.ch1.write_nr11(val),
            0xFF12 => self.ch1.write_nr12(val),
            0xFF13 => self.ch1.write_nr13(val),
            0xFF14 => self.ch1.write_nr14(val, self.frame_seq_step),
            0xFF16 => self.ch2.write_nr21(val),
            0xFF17 => self.ch2.write_nr22(val),
            0xFF18 => self.ch2.write_nr23(val),
            0xFF19 => self.ch2.write_nr24(val, self.frame_seq_step),
            0xFF1A => self.ch3.write_nr30(val),
            0xFF1B => self.ch3.write_nr31(val),
            0xFF1C => self.ch3.write_nr32(val),
            0xFF1D => self.ch3.write_nr33(val),
            0xFF1E => self.ch3.write_nr34(val, self.frame_seq_step),
            0xFF20 => self.ch4.write_nr41(val),
            0xFF21 => self.ch4.write_nr42(val),
            0xFF22 => self.ch4.write_nr43(val),
            0xFF23 => self.ch4.write_nr44(val, self.frame_seq_step),
            0xFF24 => self.nr50 = val,
            0xFF25 => self.nr51 = val,
            0xFF26 => self.write_nr52(val),
            0xFF30..=0xFF3F => self.ch3.write_wave_ram((addr - 0xFF30) as u8, val),
            _ => {}
        }
    }

    // ── NR52 ─────────────────────────────────────────────────────────────────

    pub fn powered(&self) -> bool {
        self.nr52 & 0x80 != 0
    }

    fn read_nr52(&self) -> u8 {
        let mut v = self.nr52 & 0x80; // preserve power bit
        v |= 0x70;                     // unused bits read as 1
        if self.ch1.enabled { v |= 0x01; }
        if self.ch2.enabled { v |= 0x02; }
        if self.ch3.enabled { v |= 0x04; }
        if self.ch4.enabled { v |= 0x08; }
        v
    }

    fn write_nr52(&mut self, val: u8) {
        let was_on = self.powered();
        let now_on = val & 0x80 != 0;
        self.nr52 = val & 0x80;
        if was_on && !now_on {
            // Power off: reset all registers
            self.ch1.power_off();
            self.ch2.power_off();
            self.ch3.power_off();
            self.ch4.power_off();
            self.nr50 = 0;
            self.nr51 = 0;
            self.frame_seq_step = 0;
        }
    }

    // ── Frame sequencer ──────────────────────────────────────────────────────

    /// 8-step frame sequencer at 512 Hz.
    ///
    /// ```text
    /// Step │ Length │ Sweep │ Envelope
    ///   0  │  ✓     │       │
    ///   1  │        │       │
    ///   2  │  ✓     │  ✓    │
    ///   3  │        │       │
    ///   4  │  ✓     │       │
    ///   5  │        │       │
    ///   6  │  ✓     │  ✓    │
    ///   7  │        │       │  ✓
    /// ```
    fn step_frame_sequencer(&mut self) {
        match self.frame_seq_step {
            0 | 4 => {
                self.ch1.clock_length();
                self.ch2.clock_length();
                self.ch3.clock_length();
                self.ch4.clock_length();
            }
            2 | 6 => {
                self.ch1.clock_length();
                self.ch2.clock_length();
                self.ch3.clock_length();
                self.ch4.clock_length();
                self.ch1.clock_sweep();
            }
            7 => {
                self.ch1.clock_envelope();
                self.ch2.clock_envelope();
                self.ch4.clock_envelope();
            }
            _ => {}
        }
        self.frame_seq_step = (self.frame_seq_step + 1) & 7;
    }

    // ── Mixing ───────────────────────────────────────────────────────────────

    /// Mix four 4-bit channel outputs into a stereo f32 pair and append to buf.
    fn push_sample(&mut self, c1: u8, c2: u8, c3: u8, c4: u8) {
        // NR51 panning: bits 7-4 = left, bits 3-0 = right
        // bit 7 = ch4 L, bit 6 = ch3 L, bit 5 = ch2 L, bit 4 = ch1 L
        // bit 3 = ch4 R, bit 2 = ch3 R, bit 1 = ch2 R, bit 0 = ch1 R
        let mut l: f32 = 0.0;
        let mut r: f32 = 0.0;

        let channels = [c1 as f32, c2 as f32, c3 as f32, c4 as f32];
        for (i, &ch) in channels.iter().enumerate() {
            if self.nr51 & (1 << (i + 4)) != 0 { l += ch; }
            if self.nr51 & (1 << i)       != 0 { r += ch; }
        }

        // NR50 master volume: bits 6-4 = left (0-7), bits 2-0 = right (0-7)
        let vol_l = ((self.nr50 >> 4) & 0x07) as f32 + 1.0; // 1–8
        let vol_r = (self.nr50 & 0x07) as f32 + 1.0;         // 1–8

        // Normalise: max possible sum is 4 channels * 15 amplitude * 8 volume = 480
        // We target ±1.0 peak, so divide by 60 * 8 = 480
        l = (l * vol_l) / 480.0;
        r = (r * vol_r) / 480.0;

        // High-pass filter to remove DC bias (simple leaky integrator)
        // coefficient ≈ 0.999 at 44 100 Hz → -3 dB at ~7 Hz
        self.sample_buf.push(l);
        self.sample_buf.push(r);
    }

    /// Drain the sample buffer — call once per frame after step_frame().
    pub fn drain_samples(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        std::mem::swap(&mut self.sample_buf, &mut out);
        out
    }

    /// Reset the sample buffer without returning it (for when audio is unused).
    pub fn clear_samples(&mut self) {
        self.sample_buf.clear();
    }

    // ── Debug snapshot ───────────────────────────────────────────────────────

    pub fn debug_state(&self) -> ApuDebugState {
        ApuDebugState {
            powered: self.powered(),
            nr50: self.nr50,
            nr51: self.nr51,
            nr52: self.read_nr52(),
            frame_seq_step: self.frame_seq_step,
            ch: [
                self.ch1_debug(),
                self.ch2_debug(),
                self.ch3_debug(),
                self.ch4_debug(),
            ],
            wave_ram: self.ch3.wave_ram,
        }
    }

    fn ch1_debug(&self) -> ChannelDebug {
        let freq = self.ch1.frequency();
        let hz = self.ch1.freq_hz();
        ChannelDebug {
            enabled: self.ch1.enabled,
            dac_on: self.ch1.dac_enabled,
            volume: self.ch1.env_volume,
            freq_reg: freq,
            freq_hz: hz,
            duty_or_vol: self.ch1.duty(),
            pos: self.ch1.duty_pos,
            env_add: (self.ch1.nr12 & 0x08) != 0,
            env_period: self.ch1.nr12 & 0x07,
            length: self.ch1.length_counter as u16,
            length_enabled: (self.ch1.nr14 & 0x40) != 0,
            sweep_period: (self.ch1.nr10 >> 4) & 0x07,
            sweep_shift: self.ch1.nr10 & 0x07,
            sweep_negate: (self.ch1.nr10 & 0x08) != 0,
            midi_note: freq_to_midi(hz),
            ..Default::default()
        }
    }

    fn ch2_debug(&self) -> ChannelDebug {
        let freq = self.ch2.frequency();
        let hz = self.ch2.freq_hz();
        ChannelDebug {
            enabled: self.ch2.enabled,
            dac_on: self.ch2.dac_enabled,
            volume: self.ch2.env_volume,
            freq_reg: freq,
            freq_hz: hz,
            duty_or_vol: self.ch2.duty(),
            pos: self.ch2.duty_pos,
            env_add: (self.ch2.nr22 & 0x08) != 0,
            env_period: self.ch2.nr22 & 0x07,
            length: self.ch2.length_counter as u16,
            length_enabled: (self.ch2.nr24 & 0x40) != 0,
            midi_note: freq_to_midi(hz),
            ..Default::default()
        }
    }

    fn ch3_debug(&self) -> ChannelDebug {
        let freq = self.ch3.frequency();
        let hz = self.ch3.freq_hz();
        ChannelDebug {
            enabled: self.ch3.enabled,
            dac_on: self.ch3.dac_enabled,
            volume: self.ch3.volume_code() * 5, // map 0-3 → 0,5,10,15 for UI
            freq_reg: freq,
            freq_hz: hz,
            duty_or_vol: self.ch3.volume_code(),
            pos: self.ch3.wave_pos,
            length: self.ch3.length_counter,
            length_enabled: (self.ch3.nr34 & 0x40) != 0,
            midi_note: freq_to_midi(hz),
            ..Default::default()
        }
    }

    fn ch4_debug(&self) -> ChannelDebug {
        let hz = self.ch4.freq_hz();
        ChannelDebug {
            enabled: self.ch4.enabled,
            dac_on: self.ch4.dac_enabled,
            volume: self.ch4.env_volume,
            freq_hz: hz,
            duty_or_vol: 0,
            pos: 0,
            env_add: (self.ch4.nr42 & 0x08) != 0,
            env_period: self.ch4.nr42 & 0x07,
            length: self.ch4.length_counter as u16,
            length_enabled: (self.ch4.nr44 & 0x40) != 0,
            lfsr: self.ch4.lfsr,
            lfsr_short: self.ch4.is_short_lfsr(),
            midi_note: 255, // noise has no pitch
            ..Default::default()
        }
    }
}

// ── Utility ──────────────────────────────────────────────────────────────────

/// Convert frequency in Hz to MIDI note number (A4 = 69 = 440 Hz).
/// Returns 255 if the frequency is outside the valid range.
pub fn freq_to_midi(hz: f32) -> u8 {
    if hz <= 0.0 || hz.is_nan() || hz.is_infinite() {
        return 255;
    }
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    if !(0.0..=127.0).contains(&midi) {
        return 255;
    }
    midi.round() as u8
}

/// Format a MIDI note number as a note name string (e.g. "C-4", "A#3").
/// Returns "---" for note 255 (unknown).
pub fn midi_to_note_name(note: u8) -> &'static str {
    const NAMES: [&str; 128] = [
        "C-0", "C#0", "D-0", "D#0", "E-0", "F-0", "F#0", "G-0", "G#0", "A-0", "A#0", "B-0",
        "C-1", "C#1", "D-1", "D#1", "E-1", "F-1", "F#1", "G-1", "G#1", "A-1", "A#1", "B-1",
        "C-2", "C#2", "D-2", "D#2", "E-2", "F-2", "F#2", "G-2", "G#2", "A-2", "A#2", "B-2",
        "C-3", "C#3", "D-3", "D#3", "E-3", "F-3", "F#3", "G-3", "G#3", "A-3", "A#3", "B-3",
        "C-4", "C#4", "D-4", "D#4", "E-4", "F-4", "F#4", "G-4", "G#4", "A-4", "A#4", "B-4",
        "C-5", "C#5", "D-5", "D#5", "E-5", "F-5", "F#5", "G-5", "G#5", "A-5", "A#5", "B-5",
        "C-6", "C#6", "D-6", "D#6", "E-6", "F-6", "F#6", "G-6", "G#6", "A-6", "A#6", "B-6",
        "C-7", "C#7", "D-7", "D#7", "E-7", "F-7", "F#7", "G-7", "G#7", "A-7", "A#7", "B-7",
        "C-8", "C#8", "D-8", "D#8", "E-8", "F-8", "F#8", "G-8", "G#8", "A-8", "A#8", "B-8",
        "C-9", "C#9", "D-9", "D#9", "E-9", "F-9", "F#9", "G-9", "G#9", "A-9", "A#9", "B-9",
        "C-A", "C#A", "D-A", "D#A", "E-A", "F-A", "F#A", "G-A",
    ];
    if note as usize >= NAMES.len() {
        return "---";
    }
    NAMES[note as usize]
}
