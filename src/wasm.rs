//! WASM bindings for web-based emulator.
//!
//! All emulation logic lives in [`GameBoyCore`]; this module is a thin
//! adapter that exposes it to JavaScript via `wasm-bindgen`.

use wasm_bindgen::prelude::*;

use crate::core::GameBoyCore;
use crate::log::LogCategory;
use crate::memory::io;
use crate::{log_info, log_warn};

/// Initialize panic hook for better error messages in WASM.
/// This is called once when the WASM module is instantiated.
#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct GameBoy {
    core: GameBoyCore,
}

#[wasm_bindgen]
impl GameBoy {
    #[wasm_bindgen(constructor)]
    pub fn new() -> GameBoy {
        log_info!(
            LogCategory::General,
            "GameBoy::new() - Creating emulator instance"
        );
        GameBoy {
            core: GameBoyCore::new(),
        }
    }

    pub fn load_rom(&mut self, rom_data: &[u8], cgb_mode: bool) -> Result<(), JsValue> {
        log_info!(
            LogCategory::General,
            "load_rom() - Loading ROM of {} bytes (cgb_mode={})",
            rom_data.len(),
            cgb_mode
        );

        // Log ROM header information before loading
        if rom_data.len() >= 0x150 {
            let title: String = rom_data[0x134..0x144]
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            let cart_type = rom_data[0x147];
            let rom_size = rom_data[0x148];
            let ram_size = rom_data[0x149];

            log_info!(
                LogCategory::General,
                "ROM Header: title='{}', cart_type=0x{:02X}, rom_size=0x{:02X}, ram_size=0x{:02X}",
                title,
                cart_type,
                rom_size,
                ram_size
            );

            log_info!(
                LogCategory::General,
                "Entry point (0x100-0x103): {:02X} {:02X} {:02X} {:02X}",
                rom_data[0x100],
                rom_data[0x101],
                rom_data[0x102],
                rom_data[0x103]
            );
        }

        self.core.load_rom(rom_data, cgb_mode).map_err(JsValue::from_str)?;

        log_info!(
            LogCategory::General,
            "ROM loaded successfully: {} bytes, MBC: {:?}, Banks: {}",
            rom_data.len(),
            self.core.memory.get_mbc_type(),
            self.core.memory.get_rom_bank_count()
        );

        log_info!(LogCategory::Cpu, "{}", self.core.cpu.get_debug_state());
        log_info!(LogCategory::Memory, "{}", self.core.memory.get_io_state());

        Ok(())
    }

    pub fn step_frame(&mut self) {
        let instructions_this_frame = self.core.step_frame();

        // Log state every 60 frames (approximately once per second)
        if self.core.frame_count % 60 == 1 {
            self.log_frame_debug(instructions_this_frame);
        }
    }

    pub fn frame_buffer_ptr(&self) -> *const u8 {
        self.core.frame_buffer.front().as_ptr()
    }

    pub fn frame_buffer_len(&self) -> usize {
        self.core.frame_buffer.front().len()
    }

    pub fn set_button(&mut self, button: u8, pressed: bool) {
        self.core.set_button(button, pressed);
    }

    pub fn get_cartridge_ram(&self) -> Vec<u8> {
        self.core.memory.get_cartridge_ram().to_vec()
    }

    pub fn load_cartridge_ram(&mut self, data: &[u8]) {
        self.core.memory.load_cartridge_ram(data);
    }

    /// Set camera image data from webcam.
    /// Expects 128x112 pixels as raw 8-bit grayscale (0=black, 255=white).
    pub fn set_camera_image(&mut self, data: &[u8]) {
        self.core.set_camera_image(data);
    }

    /// Check if camera image is ready for capture.
    pub fn is_camera_ready(&self) -> bool {
        self.core.is_camera_ready()
    }

    /// Check if the loaded ROM is a Game Boy Camera cartridge.
    pub fn is_camera(&self) -> bool {
        self.core.is_camera_cartridge()
    }

    /// Update the camera live view buffer if the capture has changed.
    /// Returns true if the buffer was updated.
    pub fn update_camera_live(&mut self) -> bool {
        self.core.update_camera_live()
    }

    /// Pointer to the camera live view RGBA buffer (128x112x4 bytes).
    pub fn camera_live_ptr(&self) -> *const u8 {
        self.core.camera_live_buffer.front().as_ptr()
    }

    /// Length of the camera live view buffer.
    pub fn camera_live_len(&self) -> usize {
        self.core.camera_live_buffer.front().len()
    }

    /// Decode a GB Camera saved photo slot to RGBA pixel data.
    /// Slots 1-30 = saved photos. Returns empty if slot is unoccupied.
    pub fn decode_camera_photo(&self, slot: u8) -> Vec<u8> {
        self.core.decode_camera_photo(slot)
    }

    /// Read a camera hardware register (0x00-0x7F, corresponding to A000-A07F).
    pub fn camera_reg(&self, index: u8) -> u8 {
        self.core.memory.camera_reg(index)
    }

    /// Derive the contrast level (0-15) from the current dither matrix, or -1 if unknown.
    pub fn camera_contrast(&self) -> i32 {
        self.core.memory.camera_contrast()
    }

    /// Get serial output as a string (for test ROM debugging).
    pub fn get_serial_output(&self) -> String {
        self.core.memory.get_serial_output_string()
    }

    /// Clear the serial output buffer.
    pub fn clear_serial_output(&mut self) {
        self.core.memory.clear_serial_output();
    }

    /// Get debug info about the emulator state and log to console.
    pub fn get_debug_info(&self) -> String {
        let info = format!(
            "MBC: {:?}, ROM banks: {}, LCDC: 0x{:02X}, LY: {}",
            self.core.memory.get_mbc_type(),
            self.core.memory.get_rom_bank_count(),
            self.core.memory.read_io_direct(io::LCDC),
            self.core.memory.read_io_direct(io::LY),
        );
        log_info!(LogCategory::General, "{}", info);
        info
    }

    /// Log a message to the browser console.
    pub fn log(&self, msg: &str) {
        log_info!(LogCategory::General, "{}", msg);
    }

    /// Log frame debug info.
    fn log_frame_debug(&self, instructions_this_frame: u32) {
        log_info!(
            LogCategory::General,
            "=== Frame {} | cycles: {} | instrs: {} (frame: {}) ===",
            self.core.frame_count,
            self.core.total_cycles,
            self.core.instruction_count,
            instructions_this_frame
        );

        log_info!(LogCategory::Cpu, "{}", self.core.cpu.get_debug_state());
        log_info!(LogCategory::Ppu, "{}", self.core.ppu.get_debug_state());
        log_info!(LogCategory::Memory, "{}", self.core.memory.get_io_state());
        log_info!(
            LogCategory::Memory,
            "{}",
            self.core.memory.get_debug_state()
        );

        if !self.core.memory.is_lcd_enabled() {
            log_warn!(LogCategory::General, "LCD is disabled (LCDC bit 7 = 0)");
        }

        log_info!(
            LogCategory::Ppu,
            "buffer non-zero pixels: {}",
            self.core.ppu.count_non_zero_pixels()
        );
    }

    /// Get frame count for debugging.
    pub fn get_frame_count(&self) -> u32 {
        self.core.frame_count
    }

    /// Get total instruction count for debugging.
    pub fn get_instruction_count(&self) -> u64 {
        self.core.instruction_count
    }

    /// Log detailed VRAM tile data for debugging.
    pub fn log_vram_info(&self) {
        let lcdc = self.core.memory.read_io_direct(io::LCDC);
        let tile_data_base: u16 = if lcdc & 0x10 != 0 { 0x8000 } else { 0x8800 };
        let tile_map_base: u16 = if lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };

        log_info!(
            LogCategory::Ppu,
            "VRAM: tile_data={:04X} tile_map={:04X}",
            tile_data_base,
            tile_map_base
        );

        let tile_indices: Vec<String> = (0..16)
            .map(|i| format!("{:02X}", self.core.memory.read(tile_map_base + i)))
            .collect();
        log_info!(LogCategory::Ppu, "Tile indices: {}", tile_indices.join(" "));

        let tile_data: Vec<String> = (0..16)
            .map(|i| format!("{:02X}", self.core.memory.read(0x8000 + i)))
            .collect();
        log_info!(LogCategory::Ppu, "Tile 0 data: {}", tile_data.join(" "));
    }
}

// ── Debug methods ──────────────────────────────────────────────────

#[wasm_bindgen]
impl GameBoy {
    // Execution control

    /// Execute a single CPU instruction, return cycles consumed.
    pub fn step_instruction(&mut self) -> u32 {
        self.core.step_single()
    }

    // CPU state

    pub fn cpu_pc(&self) -> u16 {
        self.core.cpu.get_debug_state().pc
    }

    pub fn cpu_sp(&self) -> u16 {
        self.core.cpu.get_debug_state().sp
    }

    pub fn cpu_a(&self) -> u8 {
        self.core.cpu.get_debug_state().a
    }

    pub fn cpu_f(&self) -> u8 {
        self.core.cpu.get_debug_state().f
    }

    pub fn cpu_bc(&self) -> u16 {
        self.core.cpu.get_debug_state().bc
    }

    pub fn cpu_de(&self) -> u16 {
        self.core.cpu.get_debug_state().de
    }

    pub fn cpu_hl(&self) -> u16 {
        self.core.cpu.get_debug_state().hl
    }

    pub fn cpu_ime(&self) -> bool {
        self.core.cpu.get_debug_state().ime
    }

    pub fn cpu_halted(&self) -> bool {
        self.core.cpu.get_debug_state().halted
    }

    // PPU state

    pub fn ppu_mode(&self) -> u8 {
        self.core.ppu.get_debug_state().mode
    }

    pub fn ppu_line(&self) -> u8 {
        self.core.ppu.get_debug_state().line
    }

    pub fn ppu_cycles(&self) -> u32 {
        self.core.ppu.get_debug_state().cycles
    }

    // Memory access

    pub fn read_byte(&self, addr: u16) -> u8 {
        self.core.memory.read(addr)
    }

    pub fn read_range(&self, addr: u16, len: u16) -> Vec<u8> {
        let mut data = Vec::with_capacity(len as usize);
        for i in 0..len {
            data.push(self.core.memory.read(addr.wrapping_add(i)));
        }
        data
    }

    /// Read bytes from VRAM at address `addr` (0x8000–0x9FFF) from an explicit bank (0 or 1).
    /// Does not modify the emulator's VBK register — safe to call at any time.
    pub fn read_vram_bank(&self, bank: u8, addr: u16, len: u16) -> Vec<u8> {
        let bank = (bank & 1) as usize;
        (0..len as u32)
            .map(|i| {
                let a = addr.wrapping_add(i as u16);
                if (0x8000..=0x9FFF).contains(&a) {
                    self.core.memory.read_vram_bank(bank, a)
                } else {
                    0xFF
                }
            })
            .collect()
    }

    // IO registers

    pub fn io_lcdc(&self) -> u8 {
        self.core.memory.read_io_direct(io::LCDC)
    }

    pub fn io_stat(&self) -> u8 {
        self.core.memory.read_io_direct(io::STAT)
    }

    pub fn io_scy(&self) -> u8 {
        self.core.memory.read_io_direct(io::SCY)
    }

    pub fn io_scx(&self) -> u8 {
        self.core.memory.read_io_direct(io::SCX)
    }

    pub fn io_ly(&self) -> u8 {
        self.core.memory.read_io_direct(io::LY)
    }

    pub fn io_lyc(&self) -> u8 {
        self.core.memory.read_io_direct(io::LYC)
    }

    pub fn io_bgp(&self) -> u8 {
        self.core.memory.read_io_direct(io::BGP)
    }

    pub fn io_obp0(&self) -> u8 {
        self.core.memory.read_io_direct(io::OBP0)
    }

    pub fn io_obp1(&self) -> u8 {
        self.core.memory.read_io_direct(io::OBP1)
    }

    pub fn io_wy(&self) -> u8 {
        self.core.memory.read_io_direct(io::WY)
    }

    pub fn io_wx(&self) -> u8 {
        self.core.memory.read_io_direct(io::WX)
    }

    pub fn io_ie(&self) -> u8 {
        self.core.memory.read(0xFFFF)
    }

    pub fn io_if(&self) -> u8 {
        self.core.memory.read_io_direct(io::IF)
    }

    pub fn io_div(&self) -> u8 {
        self.core.memory.read_io_direct(io::DIV)
    }

    pub fn io_tima(&self) -> u8 {
        self.core.memory.read_io_direct(io::TIMA)
    }

    pub fn io_tma(&self) -> u8 {
        self.core.memory.read_io_direct(io::TMA)
    }

    pub fn io_tac(&self) -> u8 {
        self.core.memory.read_io_direct(io::TAC)
    }

    pub fn io_joypad(&self) -> u8 {
        self.core.memory.read_io_direct(io::JOYP)
    }

    // ── MBC7 accelerometer ───────────────────────────────────────────────────

    pub fn is_mbc7(&self) -> bool {
        self.core.memory.get_mbc_type() == crate::memory::MbcType::Mbc7
    }

    /// Feed a new tilt reading for MBC7 (Kirby's Tilt 'n' Tumble).
    ///
    /// `x` and `y` are signed offsets from flat (0 = no tilt).
    /// Scale: ±0x1000 ≈ ±1g. The WASM host converts DeviceMotion m/s² to this unit.
    pub fn set_accelerometer(&mut self, x: i32, y: i32) {
        self.core.memory.set_accelerometer(x, y);
    }

    // ── GBC registers ────────────────────────────────────────────────────────

    pub fn is_cgb_mode(&self) -> bool {
        self.core.memory.is_cgb_mode()
    }

    /// KEY1: speed-switch register (bit 7 = current speed, bit 0 = armed).
    pub fn io_key1(&self) -> u8 {
        self.core.memory.read(0xFF4D)
    }

    /// VBK: current VRAM bank (0 or 1).
    pub fn io_vbk(&self) -> u8 {
        self.core.memory.read(0xFF4F) & 0x01
    }

    /// SVBK: current WRAM bank (1–7; bank 0 maps to 1).
    pub fn io_svbk(&self) -> u8 {
        let v = self.core.memory.read(0xFF70) & 0x07;
        if v == 0 { 1 } else { v }
    }

    /// BCPS: BG palette index register (bit 7 = auto-increment, bits 5–0 = byte address).
    pub fn io_bcps(&self) -> u8 {
        self.core.memory.read(0xFF68)
    }

    /// OCPS: OBJ palette index register (same layout as BCPS).
    pub fn io_ocps(&self) -> u8 {
        self.core.memory.read(0xFF6A)
    }

    /// OPRI: Object priority mode (bit 0: 0 = CGB coordinate order, 1 = DMG OAM order).
    pub fn io_opri(&self) -> u8 {
        self.core.memory.read(0xFF6C)
    }

    /// HDMA5: DMA status — 0xFF = idle; otherwise H-blank DMA active, bits 6–0 = remaining blocks − 1.
    pub fn io_hdma5(&self) -> u8 {
        self.core.memory.read(0xFF55)
    }

    /// Colour of `color` (0–3) in BG `palette` (0–7) as 0xRRGGBB.
    pub fn get_bg_palette_color(&self, palette: u8, color: u8) -> u32 {
        let (lo, hi) = self.core.memory.read_bg_palette(palette as usize, color as usize);
        rgb555_to_rgb888(lo, hi)
    }

    /// Colour of `color` (0–3) in OBJ `palette` (0–7) as 0xRRGGBB.
    pub fn get_obj_palette_color(&self, palette: u8, color: u8) -> u32 {
        let (lo, hi) = self.core.memory.read_obj_palette(palette as usize, color as usize);
        rgb555_to_rgb888(lo, hi)
    }
}

// ── APU / Audio ──────────────────────────────────────────────────────────────

#[wasm_bindgen]
impl GameBoy {
    // ── Audio sample buffer ──────────────────────────────────────────────────

    /// Pointer to the interleaved stereo f32 sample buffer (L, R, L, R, …).
    /// Valid until the next call to `step_frame`.
    pub fn audio_sample_buffer_ptr(&self) -> *const f32 {
        self.core.apu.sample_buf.as_ptr()
    }

    /// Number of f32 values in the sample buffer (pairs of L/R samples).
    pub fn audio_sample_buffer_len(&self) -> usize {
        self.core.apu.sample_buf.len()
    }

    /// Target sample rate in Hz (44 100).
    pub fn audio_sample_rate(&self) -> u32 {
        crate::apu::SAMPLE_RATE
    }

    /// Clear the sample buffer after the host has consumed it.
    pub fn audio_clear_samples(&mut self) {
        self.core.apu.clear_samples();
    }

    // ── APU control registers ────────────────────────────────────────────────

    pub fn apu_powered(&self) -> bool  { self.core.apu.powered() }
    pub fn apu_nr50(&self)    -> u8    { self.core.apu.nr50 }
    pub fn apu_nr51(&self)    -> u8    { self.core.apu.nr51 }
    pub fn apu_nr52(&self)    -> u8    { self.core.apu.read(0xFF26) }

    // ── CH1 (Pulse + Sweep) ──────────────────────────────────────────────────

    pub fn apu_ch1_enabled(&self)    -> bool  { self.core.apu.ch1.enabled }
    pub fn apu_ch1_dac(&self)        -> bool  { self.core.apu.ch1.dac_enabled }
    pub fn apu_ch1_volume(&self)     -> u8    { self.core.apu.ch1.env_volume }
    pub fn apu_ch1_freq_reg(&self)   -> u16   { self.core.apu.ch1.frequency() }
    pub fn apu_ch1_freq_hz(&self)    -> f32   { self.core.apu.ch1.freq_hz() }
    pub fn apu_ch1_duty(&self)       -> u8    { self.core.apu.ch1.duty() }
    pub fn apu_ch1_duty_pos(&self)   -> u8    { self.core.apu.ch1.duty_pos }
    pub fn apu_ch1_length(&self)     -> u8    { self.core.apu.ch1.length_counter }
    pub fn apu_ch1_len_en(&self)     -> bool  { (self.core.apu.ch1.nr14 & 0x40) != 0 }
    pub fn apu_ch1_env_add(&self)    -> bool  { (self.core.apu.ch1.nr12 & 0x08) != 0 }
    pub fn apu_ch1_env_period(&self) -> u8    { self.core.apu.ch1.nr12 & 0x07 }
    pub fn apu_ch1_sweep_period(&self) -> u8  { (self.core.apu.ch1.nr10 >> 4) & 0x07 }
    pub fn apu_ch1_sweep_shift(&self)  -> u8  { self.core.apu.ch1.nr10 & 0x07 }
    pub fn apu_ch1_sweep_neg(&self)    -> bool { (self.core.apu.ch1.nr10 & 0x08) != 0 }
    pub fn apu_ch1_shadow_freq(&self)  -> u16  { self.core.apu.ch1.shadow_freq }
    /// MIDI note number for CH1 frequency (255 = unknown/out of range).
    pub fn apu_ch1_midi_note(&self) -> u8 {
        crate::apu::freq_to_midi(self.core.apu.ch1.freq_hz())
    }

    // ── CH2 (Pulse) ──────────────────────────────────────────────────────────

    pub fn apu_ch2_enabled(&self)    -> bool  { self.core.apu.ch2.enabled }
    pub fn apu_ch2_dac(&self)        -> bool  { self.core.apu.ch2.dac_enabled }
    pub fn apu_ch2_volume(&self)     -> u8    { self.core.apu.ch2.env_volume }
    pub fn apu_ch2_freq_reg(&self)   -> u16   { self.core.apu.ch2.frequency() }
    pub fn apu_ch2_freq_hz(&self)    -> f32   { self.core.apu.ch2.freq_hz() }
    pub fn apu_ch2_duty(&self)       -> u8    { self.core.apu.ch2.duty() }
    pub fn apu_ch2_duty_pos(&self)   -> u8    { self.core.apu.ch2.duty_pos }
    pub fn apu_ch2_length(&self)     -> u8    { self.core.apu.ch2.length_counter }
    pub fn apu_ch2_len_en(&self)     -> bool  { (self.core.apu.ch2.nr24 & 0x40) != 0 }
    pub fn apu_ch2_env_add(&self)    -> bool  { (self.core.apu.ch2.nr22 & 0x08) != 0 }
    pub fn apu_ch2_env_period(&self) -> u8    { self.core.apu.ch2.nr22 & 0x07 }
    pub fn apu_ch2_midi_note(&self)  -> u8 {
        crate::apu::freq_to_midi(self.core.apu.ch2.freq_hz())
    }

    // ── CH3 (Wave) ───────────────────────────────────────────────────────────

    pub fn apu_ch3_enabled(&self)    -> bool  { self.core.apu.ch3.enabled }
    pub fn apu_ch3_dac(&self)        -> bool  { self.core.apu.ch3.dac_enabled }
    pub fn apu_ch3_vol_code(&self)   -> u8    { self.core.apu.ch3.volume_code() }
    pub fn apu_ch3_freq_reg(&self)   -> u16   { self.core.apu.ch3.frequency() }
    pub fn apu_ch3_freq_hz(&self)    -> f32   { self.core.apu.ch3.freq_hz() }
    pub fn apu_ch3_wave_pos(&self)   -> u8    { self.core.apu.ch3.wave_pos }
    pub fn apu_ch3_length(&self)     -> u16   { self.core.apu.ch3.length_counter }
    pub fn apu_ch3_len_en(&self)     -> bool  { (self.core.apu.ch3.nr34 & 0x40) != 0 }
    /// Raw wave RAM as 16 bytes (32 × 4-bit nibbles).
    pub fn apu_ch3_wave_ram(&self) -> Vec<u8> { self.core.apu.ch3.wave_ram.to_vec() }
    pub fn apu_ch3_midi_note(&self) -> u8 {
        crate::apu::freq_to_midi(self.core.apu.ch3.freq_hz())
    }

    // ── CH4 (Noise) ──────────────────────────────────────────────────────────

    pub fn apu_ch4_enabled(&self)    -> bool  { self.core.apu.ch4.enabled }
    pub fn apu_ch4_dac(&self)        -> bool  { self.core.apu.ch4.dac_enabled }
    pub fn apu_ch4_volume(&self)     -> u8    { self.core.apu.ch4.env_volume }
    pub fn apu_ch4_freq_hz(&self)    -> f32   { self.core.apu.ch4.freq_hz() }
    pub fn apu_ch4_clock_shift(&self) -> u8   { self.core.apu.ch4.clock_shift() }
    pub fn apu_ch4_clock_div(&self)   -> u8   { self.core.apu.ch4.clock_divider() }
    pub fn apu_ch4_lfsr_short(&self)  -> bool { self.core.apu.ch4.is_short_lfsr() }
    pub fn apu_ch4_lfsr(&self)        -> u16  { self.core.apu.ch4.lfsr }
    pub fn apu_ch4_length(&self)      -> u8   { self.core.apu.ch4.length_counter }
    pub fn apu_ch4_len_en(&self)      -> bool { (self.core.apu.ch4.nr44 & 0x40) != 0 }
    pub fn apu_ch4_env_add(&self)     -> bool { (self.core.apu.ch4.nr42 & 0x08) != 0 }
    pub fn apu_ch4_env_period(&self)  -> u8   { self.core.apu.ch4.nr42 & 0x07 }

    // ── Frame sequencer / note name ──────────────────────────────────────────

    /// Current frame sequencer step (0–7).  Drives length/sweep/envelope clocks.
    pub fn apu_frame_seq_step(&self) -> u8 {
        self.core.apu.debug_state().frame_seq_step
    }

    /// Convert a MIDI note number (0–127) to a note name string like "C-4" or "A#3".
    pub fn midi_to_note_name(note: u8) -> String {
        crate::apu::midi_to_note_name(note).to_string()
    }
}

/// Convert RGB555 (lo byte, hi byte) to 0xRRGGBB.
fn rgb555_to_rgb888(lo: u8, hi: u8) -> u32 {
    let raw = (lo as u16) | ((hi as u16) << 8);
    let r5 = (raw & 0x1F) as u32;
    let g5 = ((raw >> 5) & 0x1F) as u32;
    let b5 = ((raw >> 10) & 0x1F) as u32;
    // Expand 5→8 bits: multiply by 255/31 ≈ 8.226; shifting left 3 is a fast approximation.
    let r8 = (r5 << 3) | (r5 >> 2);
    let g8 = (g5 << 3) | (g5 >> 2);
    let b8 = (b5 << 3) | (b5 >> 2);
    (r8 << 16) | (g8 << 8) | b8
}

impl Default for GameBoy {
    fn default() -> Self {
        Self::new()
    }
}
