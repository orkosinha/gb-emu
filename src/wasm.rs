//! WASM bindings for the web-based emulator frontend.
//!
//! All emulation logic lives in [`GameBoyCore`]; this module is a thin
//! adapter that exposes it to JavaScript via `wasm-bindgen`.
//!
//! ## Features
//! - Default (`wasm`): production API — lifecycle, input, video, audio, save data.
//! - `debug`: adds instruction stepping, register inspection, and per-channel
//!   APU state.  Not intended for production deployments.

use wasm_bindgen::prelude::*;

use crate::core::GameBoyCore;
use crate::joypad::Button;
use crate::log::LogCategory;
use crate::memory::io;
use crate::log_info;

/// Initialize panic hook for better error messages in WASM.
/// Called automatically when the WASM module is instantiated.
#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct GameBoy {
    core: GameBoyCore,
}

// ── Production API ────────────────────────────────────────────────────────────

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
        if rom_data.len() >= 0x150 {
            let title: String = rom_data[0x134..0x144]
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            log_info!(
                LogCategory::General,
                "load_rom: title='{}', cart_type=0x{:02X}, rom_size=0x{:02X}, ram_size=0x{:02X}",
                title,
                rom_data[0x147],
                rom_data[0x148],
                rom_data[0x149],
            );
        }

        self.core
            .load_rom(rom_data, cgb_mode)
            .map_err(JsValue::from_str)?;

        log_info!(
            LogCategory::General,
            "ROM loaded: {} bytes, MBC: {:?}, banks: {}",
            rom_data.len(),
            self.core.memory.get_mbc_type(),
            self.core.memory.get_rom_bank_count(),
        );
        Ok(())
    }

    /// Run one full frame of emulation (~16.74 ms of Game Boy time).
    pub fn step_frame(&mut self) {
        self.core.step_frame();
    }

    /// Run until at least `n` stereo sample pairs are in the audio buffer.
    /// Returns the number of pairs actually generated.
    pub fn step_samples(&mut self, n: usize) -> usize {
        self.core.step_samples(n)
    }

    /// Power-cycle: resets CPU, PPU, APU, timer, and MBC banking state.
    /// Save RAM is cleared; call `load_cartridge_ram` afterwards to restore it.
    pub fn reset(&mut self) {
        self.core.reset();
    }

    // ── Video ────────────────────────────────────────────────────────────────

    /// Pointer to the front frame buffer (160×144 RGBA pixels).
    /// Valid until the next `step_frame` call.
    pub fn frame_buffer_ptr(&self) -> *const u8 {
        self.core.frame_buffer.front().as_ptr()
    }

    pub fn frame_buffer_len(&self) -> usize {
        self.core.frame_buffer.front().len()
    }

    // ── Input ────────────────────────────────────────────────────────────────

    pub fn set_button(&mut self, button: Button, pressed: bool) {
        self.core.set_button(button as u8, pressed);
    }

    // ── Save data (cartridge SRAM) ────────────────────────────────────────────
    //
    // Two read paths:
    //   • `get_cartridge_ram()` — copies RAM into a JS-owned Vec (simple).
    //   • `get_cartridge_ram_ptr/len` — zero-copy view into the emulator's
    //     backing store.  The pointer is invalidated by any call that mutates
    //     the cartridge (write_cartridge_ram, reset, load_rom, load_cartridge_ram).
    //
    // Two write paths:
    //   • `load_cartridge_ram(data)` — bulk replace (e.g. loading a .sav file).
    //   • `write_cartridge_ram/write_cartridge_ram_range` — partial flat-offset
    //     writes that bypass MBC bank logic, for live SRAM patching.

    pub fn get_cartridge_ram(&self) -> Vec<u8> {
        self.core.memory.get_cartridge_ram().to_vec()
    }

    pub fn get_cartridge_ram_ptr(&self) -> *const u8 {
        self.core.memory.get_cartridge_ram().as_ptr()
    }

    pub fn get_cartridge_ram_len(&self) -> usize {
        self.core.memory.get_cartridge_ram().len()
    }

    pub fn load_cartridge_ram(&mut self, data: &[u8]) {
        self.core.memory.load_cartridge_ram(data);
    }

    /// Write one byte to cartridge SRAM at flat `.sav` offset `offset`,
    /// bypassing MBC bank selection.
    pub fn write_cartridge_ram(&mut self, offset: usize, value: u8) {
        self.core.memory.write_cartridge_ram_flat(offset, value);
    }

    /// Write a contiguous slice to cartridge SRAM starting at flat offset `offset`.
    pub fn write_cartridge_ram_range(&mut self, offset: usize, data: &[u8]) {
        self.core.memory.write_cartridge_ram_range_flat(offset, data);
    }

    // ── Serial port ──────────────────────────────────────────────────────────

    /// Deliver one byte as if sent by an external device on the serial link.
    /// Fires the serial interrupt (IF bit 3).
    pub fn serial_receive(&mut self, byte: u8) {
        self.core.serial_inject(byte);
    }

    /// Return and consume the oldest byte transmitted on the serial port,
    /// or `undefined` if nothing was sent.
    pub fn serial_take_output(&mut self) -> Option<u8> {
        self.core.serial_take_output()
    }

    // ── Camera ───────────────────────────────────────────────────────────────

    /// Set camera image data from a live capture source.
    /// Expects 128×112 pixels as raw 8-bit grayscale (0 = black, 255 = white).
    pub fn set_camera_image(&mut self, data: &[u8]) {
        self.core.set_camera_image(data);
    }

    pub fn is_camera_ready(&self) -> bool {
        self.core.is_camera_ready()
    }

    pub fn is_camera(&self) -> bool {
        self.core.is_camera_cartridge()
    }

    /// Update the camera live-view buffer. Returns true if the buffer changed.
    pub fn update_camera_live(&mut self) -> bool {
        self.core.update_camera_live()
    }

    /// Pointer to the camera live-view RGBA buffer (128×112×4 bytes).
    pub fn camera_live_ptr(&self) -> *const u8 {
        self.core.camera_live_buffer.front().as_ptr()
    }

    pub fn camera_live_len(&self) -> usize {
        self.core.camera_live_buffer.front().len()
    }

    /// Decode a saved photo slot (1–30) to RGBA. Returns empty if unoccupied.
    pub fn decode_camera_photo(&self, slot: u8) -> Vec<u8> {
        self.core.decode_camera_photo(slot)
    }

    /// Read a camera hardware register (index 0x00–0x7F → addresses 0xA000–0xA07F).
    pub fn camera_reg(&self, index: u8) -> u8 {
        self.core.memory.camera_reg(index)
    }

    /// Current contrast level derived from the dither matrix (0–15, or −1 if unknown).
    pub fn camera_contrast(&self) -> i32 {
        self.core.memory.camera_contrast()
    }

    // ── Audio ────────────────────────────────────────────────────────────────

    /// Pointer to the interleaved stereo f32 sample buffer (L, R, L, R, …).
    /// Valid until the next `step_frame` or `step_samples` call.
    pub fn audio_sample_buffer_ptr(&self) -> *const f32 {
        self.core.apu.sample_buf.as_ptr()
    }

    /// Number of f32 values in the sample buffer (pairs × 2).
    pub fn audio_sample_buffer_len(&self) -> usize {
        self.core.apu.sample_buf.len()
    }

    /// Target sample rate in Hz (44 100).
    pub fn audio_sample_rate(&self) -> u32 {
        crate::apu::SAMPLE_RATE
    }

    /// Discard all samples from the buffer. Call once per frame after consuming.
    pub fn audio_clear_samples(&mut self) {
        self.core.apu.clear_samples();
    }

    pub fn apu_powered(&self) -> bool {
        self.core.apu.powered()
    }

    /// NR50: master volume and VIN panning register.
    pub fn apu_nr50(&self) -> u8 {
        self.core.apu.nr50
    }

    /// NR51: left/right channel output routing mask.
    pub fn apu_nr51(&self) -> u8 {
        self.core.apu.nr51
    }

    /// NR52: APU power and channel-active status bits.
    pub fn apu_nr52(&self) -> u8 {
        self.core.apu.read(0xFF26)
    }

    // ── MBC7 accelerometer ───────────────────────────────────────────────────

    pub fn is_mbc7(&self) -> bool {
        self.core.memory.get_mbc_type() == crate::memory::MbcType::Mbc7
    }

    /// Feed a tilt reading to the MBC7 accelerometer.
    /// `x`/`y` are signed offsets from flat; ±0x1000 ≈ ±1g.
    pub fn set_accelerometer(&mut self, x: i32, y: i32) {
        self.core.memory.set_accelerometer(x, y);
    }

    // ── GBC ──────────────────────────────────────────────────────────────────

    pub fn is_cgb_mode(&self) -> bool {
        self.core.memory.is_cgb_mode()
    }

    /// Colour of `color` (0–3) in BG palette `palette` (0–7) as 0xRRGGBB.
    pub fn get_bg_palette_color(&self, palette: u8, color: u8) -> u32 {
        let (lo, hi) = self
            .core
            .memory
            .read_bg_palette(palette as usize, color as usize);
        rgb555_to_rgb888(lo, hi)
    }

    /// Colour of `color` (0–3) in OBJ palette `palette` (0–7) as 0xRRGGBB.
    pub fn get_obj_palette_color(&self, palette: u8, color: u8) -> u32 {
        let (lo, hi) = self
            .core
            .memory
            .read_obj_palette(palette as usize, color as usize);
        rgb555_to_rgb888(lo, hi)
    }

    // ── Diagnostics ──────────────────────────────────────────────────────────

    /// Lightweight status string: MBC type, ROM banks, LCDC, LY.
    pub fn get_debug_info(&self) -> String {
        format!(
            "MBC: {:?}, ROM banks: {}, LCDC: 0x{:02X}, LY: {}",
            self.core.memory.get_mbc_type(),
            self.core.memory.get_rom_bank_count(),
            self.core.memory.read_io_direct(io::LCDC),
            self.core.memory.read_io_direct(io::LY),
        )
    }

    pub fn get_frame_count(&self) -> u32 {
        self.core.frame_count
    }

    pub fn get_instruction_count(&self) -> u64 {
        self.core.instruction_count
    }

    /// Serial output as a string (useful for test ROM output on 0xFF01/0xFF02).
    pub fn get_serial_output(&self) -> String {
        self.core.memory.get_serial_output_string()
    }

    pub fn clear_serial_output(&mut self) {
        self.core.memory.clear_serial_output();
    }

    /// Convert a MIDI note number (0–127) to a name string like "C-4" or "A#3".
    pub fn midi_to_note_name(note: u8) -> String {
        crate::apu::midi_to_note_name(note).to_string()
    }
}

// ── Debug API ─────────────────────────────────────────────────────────────────
//
// Compile with `--features debug` to enable.  Not intended for production.

#[cfg(feature = "debug")]
#[wasm_bindgen]
impl GameBoy {
    // ── Execution control ─────────────────────────────────────────────────────

    /// Execute a single CPU instruction. Returns T-cycles consumed.
    pub fn step_instruction(&mut self) -> u32 {
        self.core.step_single()
    }

    // ── CPU registers ─────────────────────────────────────────────────────────

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

    // ── PPU ───────────────────────────────────────────────────────────────────

    pub fn ppu_mode(&self) -> u8 {
        self.core.ppu.get_debug_state().mode
    }
    pub fn ppu_line(&self) -> u8 {
        self.core.ppu.get_debug_state().line
    }
    pub fn ppu_cycles(&self) -> u32 {
        self.core.ppu.get_debug_state().cycles
    }

    // ── Memory inspection ─────────────────────────────────────────────────────

    pub fn read_byte(&self, addr: u16) -> u8 {
        self.core.memory.read(addr)
    }

    pub fn read_range(&self, addr: u16, len: u16) -> Vec<u8> {
        (0..len)
            .map(|i| self.core.memory.read(addr.wrapping_add(i)))
            .collect()
    }

    /// Read `len` bytes from VRAM bank `bank` (0 or 1) at `addr` (0x8000–0x9FFF).
    /// Does not modify VBK — safe to call at any time.
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

    // ── IO registers (DMG) ────────────────────────────────────────────────────

    pub fn io_lcdc(&self) -> u8 { self.core.memory.read_io_direct(io::LCDC) }
    pub fn io_stat(&self) -> u8 { self.core.memory.read_io_direct(io::STAT) }
    pub fn io_scy(&self)  -> u8 { self.core.memory.read_io_direct(io::SCY)  }
    pub fn io_scx(&self)  -> u8 { self.core.memory.read_io_direct(io::SCX)  }
    pub fn io_ly(&self)   -> u8 { self.core.memory.read_io_direct(io::LY)   }
    pub fn io_lyc(&self)  -> u8 { self.core.memory.read_io_direct(io::LYC)  }
    pub fn io_bgp(&self)  -> u8 { self.core.memory.read_io_direct(io::BGP)  }
    pub fn io_obp0(&self) -> u8 { self.core.memory.read_io_direct(io::OBP0) }
    pub fn io_obp1(&self) -> u8 { self.core.memory.read_io_direct(io::OBP1) }
    pub fn io_wy(&self)   -> u8 { self.core.memory.read_io_direct(io::WY)   }
    pub fn io_wx(&self)   -> u8 { self.core.memory.read_io_direct(io::WX)   }
    pub fn io_ie(&self)   -> u8 { self.core.memory.read(0xFFFF)              }
    pub fn io_if(&self)   -> u8 { self.core.memory.read_io_direct(io::IF)   }
    pub fn io_div(&self)  -> u8 { self.core.memory.read_io_direct(io::DIV)  }
    pub fn io_tima(&self) -> u8 { self.core.memory.read_io_direct(io::TIMA) }
    pub fn io_tma(&self)  -> u8 { self.core.memory.read_io_direct(io::TMA)  }
    pub fn io_tac(&self)  -> u8 { self.core.memory.read_io_direct(io::TAC)  }
    pub fn io_joypad(&self) -> u8 { self.core.memory.read_io_direct(io::JOYP) }

    // ── IO registers (GBC) ────────────────────────────────────────────────────

    /// KEY1: speed-switch (bit 7 = current speed, bit 0 = armed).
    pub fn io_key1(&self) -> u8 { self.core.memory.read(0xFF4D) }
    /// VBK: current VRAM bank (0 or 1).
    pub fn io_vbk(&self) -> u8 { self.core.memory.read(0xFF4F) & 0x01 }
    /// SVBK: current WRAM bank (1–7; 0 maps to 1).
    pub fn io_svbk(&self) -> u8 {
        let v = self.core.memory.read(0xFF70) & 0x07;
        if v == 0 { 1 } else { v }
    }
    /// BCPS: BG palette index (bit 7 = auto-increment, bits 5–0 = byte address).
    pub fn io_bcps(&self) -> u8 { self.core.memory.read(0xFF68) }
    /// OCPS: OBJ palette index (same layout as BCPS).
    pub fn io_ocps(&self) -> u8 { self.core.memory.read(0xFF6A) }
    /// OPRI: Object priority mode (bit 0: 0 = CGB coordinate, 1 = DMG OAM order).
    pub fn io_opri(&self) -> u8 { self.core.memory.read(0xFF6C) }
    /// HDMA5: 0xFF = idle; otherwise H-blank DMA active, bits 6–0 = remaining blocks − 1.
    pub fn io_hdma5(&self) -> u8 { self.core.memory.read(0xFF55) }

    // ── APU per-channel state ─────────────────────────────────────────────────
    //
    // All accessors go through the typed ChannelDebug snapshot so that raw
    // register layouts (nr10, nr12, nr14 …) stay inside the APU module.

    pub fn apu_frame_seq_step(&self) -> u8 {
        self.core.apu.debug_state().frame_seq_step
    }

    // CH1
    pub fn apu_ch1_enabled(&self) -> bool  { self.core.apu.ch1_debug().enabled }
    pub fn apu_ch1_dac(&self) -> bool       { self.core.apu.ch1_debug().dac_on }
    pub fn apu_ch1_volume(&self) -> u8      { self.core.apu.ch1_debug().volume }
    pub fn apu_ch1_freq_reg(&self) -> u16   { self.core.apu.ch1_debug().freq_reg }
    pub fn apu_ch1_freq_hz(&self) -> f32    { self.core.apu.ch1_debug().freq_hz }
    pub fn apu_ch1_duty(&self) -> u8        { self.core.apu.ch1_debug().duty_or_vol }
    pub fn apu_ch1_duty_pos(&self) -> u8    { self.core.apu.ch1_debug().pos }
    pub fn apu_ch1_length(&self) -> u8      { self.core.apu.ch1_debug().length as u8 }
    pub fn apu_ch1_len_en(&self) -> bool    { self.core.apu.ch1_debug().length_enabled }
    pub fn apu_ch1_env_add(&self) -> bool   { self.core.apu.ch1_debug().env_add }
    pub fn apu_ch1_env_period(&self) -> u8  { self.core.apu.ch1_debug().env_period }
    pub fn apu_ch1_sweep_period(&self) -> u8 { self.core.apu.ch1_debug().sweep_period }
    pub fn apu_ch1_sweep_shift(&self) -> u8  { self.core.apu.ch1_debug().sweep_shift }
    pub fn apu_ch1_sweep_neg(&self) -> bool  { self.core.apu.ch1_debug().sweep_negate }
    pub fn apu_ch1_midi_note(&self) -> u8   { self.core.apu.ch1_debug().midi_note }

    // CH2
    pub fn apu_ch2_enabled(&self) -> bool  { self.core.apu.ch2_debug().enabled }
    pub fn apu_ch2_dac(&self) -> bool       { self.core.apu.ch2_debug().dac_on }
    pub fn apu_ch2_volume(&self) -> u8      { self.core.apu.ch2_debug().volume }
    pub fn apu_ch2_freq_reg(&self) -> u16   { self.core.apu.ch2_debug().freq_reg }
    pub fn apu_ch2_freq_hz(&self) -> f32    { self.core.apu.ch2_debug().freq_hz }
    pub fn apu_ch2_duty(&self) -> u8        { self.core.apu.ch2_debug().duty_or_vol }
    pub fn apu_ch2_duty_pos(&self) -> u8    { self.core.apu.ch2_debug().pos }
    pub fn apu_ch2_length(&self) -> u8      { self.core.apu.ch2_debug().length as u8 }
    pub fn apu_ch2_len_en(&self) -> bool    { self.core.apu.ch2_debug().length_enabled }
    pub fn apu_ch2_env_add(&self) -> bool   { self.core.apu.ch2_debug().env_add }
    pub fn apu_ch2_env_period(&self) -> u8  { self.core.apu.ch2_debug().env_period }
    pub fn apu_ch2_midi_note(&self) -> u8   { self.core.apu.ch2_debug().midi_note }

    // CH3
    pub fn apu_ch3_enabled(&self) -> bool  { self.core.apu.ch3_debug().enabled }
    pub fn apu_ch3_dac(&self) -> bool       { self.core.apu.ch3_debug().dac_on }
    pub fn apu_ch3_vol_code(&self) -> u8    { self.core.apu.ch3_debug().duty_or_vol }
    pub fn apu_ch3_freq_reg(&self) -> u16   { self.core.apu.ch3_debug().freq_reg }
    pub fn apu_ch3_freq_hz(&self) -> f32    { self.core.apu.ch3_debug().freq_hz }
    pub fn apu_ch3_wave_pos(&self) -> u8    { self.core.apu.ch3_debug().pos }
    pub fn apu_ch3_length(&self) -> u16     { self.core.apu.ch3_debug().length }
    pub fn apu_ch3_len_en(&self) -> bool    { self.core.apu.ch3_debug().length_enabled }
    pub fn apu_ch3_midi_note(&self) -> u8   { self.core.apu.ch3_debug().midi_note }
    /// Raw wave RAM as 16 bytes (32 × 4-bit nibbles).
    pub fn apu_ch3_wave_ram(&self) -> Vec<u8> {
        self.core.apu.ch3.wave_ram.to_vec()
    }

    // CH4
    pub fn apu_ch4_enabled(&self) -> bool  { self.core.apu.ch4_debug().enabled }
    pub fn apu_ch4_dac(&self) -> bool       { self.core.apu.ch4_debug().dac_on }
    pub fn apu_ch4_volume(&self) -> u8      { self.core.apu.ch4_debug().volume }
    pub fn apu_ch4_freq_hz(&self) -> f32    { self.core.apu.ch4_debug().freq_hz }
    pub fn apu_ch4_length(&self) -> u8      { self.core.apu.ch4_debug().length as u8 }
    pub fn apu_ch4_len_en(&self) -> bool    { self.core.apu.ch4_debug().length_enabled }
    pub fn apu_ch4_env_add(&self) -> bool   { self.core.apu.ch4_debug().env_add }
    pub fn apu_ch4_env_period(&self) -> u8  { self.core.apu.ch4_debug().env_period }
    pub fn apu_ch4_lfsr(&self) -> u16       { self.core.apu.ch4_debug().lfsr }
    pub fn apu_ch4_lfsr_short(&self) -> bool { self.core.apu.ch4_debug().lfsr_short }
    // CH4 clock parameters derived from NR43 — expose them through the channel directly.
    pub fn apu_ch4_clock_shift(&self) -> u8  { self.core.apu.ch4.clock_shift() }
    pub fn apu_ch4_clock_div(&self) -> u8    { self.core.apu.ch4.clock_divider() }

    // ── APU visualization ─────────────────────────────────────────────────────

    /// Pointer to the per-channel visualization ring buffer.
    /// Layout: 4 channels × 512 bytes; channel c starts at byte c × 512.
    pub fn apu_viz_ptr(&self) -> *const u8 {
        self.core.apu.viz_buf.as_ptr()
    }

    /// Current write index within each channel's 512-byte slot (0–511).
    pub fn apu_viz_wp(&self) -> usize {
        self.core.apu.viz_wp
    }

    // ── Shadow freq (CH1 sweep) ───────────────────────────────────────────────

    /// CH1 shadow frequency register (internal sweep calculation state).
    pub fn apu_ch1_shadow_freq(&self) -> u16 {
        self.core.apu.ch1.shadow_freq
    }

    // ── Logging ───────────────────────────────────────────────────────────────

    pub fn log(&self, msg: &str) {
        log_info!(LogCategory::General, "{}", msg);
    }

    pub fn log_vram_info(&self) {
        let lcdc = self.core.memory.read_io_direct(io::LCDC);
        let tile_data_base: u16 = if lcdc & 0x10 != 0 { 0x8000 } else { 0x8800 };
        let tile_map_base: u16 = if lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };
        log_info!(
            LogCategory::Ppu,
            "VRAM: tile_data=0x{:04X} tile_map=0x{:04X}",
            tile_data_base,
            tile_map_base,
        );
        let tile_indices: Vec<String> = (0..16u16)
            .map(|i| format!("{:02X}", self.core.memory.read(tile_map_base + i)))
            .collect();
        log_info!(LogCategory::Ppu, "Tile indices[0..15]: {}", tile_indices.join(" "));
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert RGB555 (lo byte, hi byte little-endian) to 0xRRGGBB.
fn rgb555_to_rgb888(lo: u8, hi: u8) -> u32 {
    let raw = (lo as u16) | ((hi as u16) << 8);
    let r5 = (raw & 0x1F) as u32;
    let g5 = ((raw >> 5) & 0x1F) as u32;
    let b5 = ((raw >> 10) & 0x1F) as u32;
    // Expand 5→8 bits: (x << 3) | (x >> 2) gives the full 0–255 range.
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
