//! Shared emulator core.
//!
//! [`GameBoyCore`] owns all emulator components and provides the main
//! `step_frame` loop, ROM loading, button input, and camera integration.

use crate::apu::Apu;
use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::interrupts::{Interrupt, InterruptController};
use crate::joypad::Joypad;
use crate::memory::Memory;
use crate::ppu::Ppu;
use crate::snapshot::{SnapReader, SnapWriter, Snapshot};
use crate::timer::Timer;

const CYCLES_PER_FRAME: u32 = 70_224;
const CYCLES_PER_FRAME_DOUBLE: u32 = 140_448; // CPU runs 2× but PPU timing unchanged
const FRAME_BUFFER_SIZE: usize = 160 * 144 * 4;
const CAMERA_BUFFER_SIZE: usize = 128 * 112 * 4;

pub struct DoubleBuffer<const N: usize> {
    buffers: [Box<[u8; N]>; 2],
    front: usize,
}

impl<const N: usize> Default for DoubleBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> DoubleBuffer<N> {
    pub fn new() -> Self {
        DoubleBuffer {
            buffers: [Box::new([0u8; N]), Box::new([0u8; N])],
            front: 0,
        }
    }

    #[inline]
    pub fn front(&self) -> &[u8; N] {
        &self.buffers[self.front]
    }

    #[inline]
    pub fn back_mut(&mut self) -> &mut [u8; N] {
        &mut self.buffers[1 - self.front]
    }

    #[inline]
    pub fn swap(&mut self) {
        self.front = 1 - self.front;
    }
}

pub struct GameBoyCore {
    pub(crate) cpu: Cpu,
    pub(crate) memory: Memory,
    pub(crate) ppu: Ppu,
    pub(crate) timer: Timer,
    pub(crate) apu: Apu,
    pub(crate) interrupts: InterruptController,
    pub(crate) joypad: Joypad,
    pub(crate) frame_buffer: DoubleBuffer<FRAME_BUFFER_SIZE>,
    pub(crate) camera_live_buffer: DoubleBuffer<CAMERA_BUFFER_SIZE>,
    pub(crate) frame_count: u32,
    pub(crate) total_cycles: u64,
    pub(crate) instruction_count: u64,
}

impl Default for GameBoyCore {
    fn default() -> Self {
        Self::new()
    }
}

impl GameBoyCore {
    pub fn new() -> Self {
        GameBoyCore {
            cpu: Cpu::new(),
            memory: Memory::new(),
            ppu: Ppu::new(),
            timer: Timer::new(),
            apu: Apu::new(),
            interrupts: InterruptController::new(),
            joypad: Joypad::new(),
            frame_buffer: DoubleBuffer::new(),
            camera_live_buffer: DoubleBuffer::new(),
            frame_count: 0,
            total_cycles: 0,
            instruction_count: 0,
        }
    }

    pub fn load_rom(&mut self, rom_data: &[u8], cgb_mode: bool) -> Result<(), &'static str> {
        self.memory.load_rom(rom_data, cgb_mode)?;
        self.reset_components(cgb_mode);
        Ok(())
    }

    /// Power-cycle the emulator: resets CPU, PPU, APU, timer, and MBC banking
    /// state to power-on defaults.  Save RAM is cleared; call `load_cartridge_ram`
    /// after this to restore a previous save.
    pub fn reset(&mut self) {
        let cgb_mode = self.memory.is_cgb_mode();
        self.memory.reset_hardware();
        self.reset_components(cgb_mode);
    }

    fn reset_components(&mut self, cgb_mode: bool) {
        self.cpu.reset(cgb_mode);
        self.ppu.reset(cgb_mode);
        self.timer = crate::timer::Timer::new();
        self.apu = crate::apu::Apu::new();
        self.interrupts = crate::interrupts::InterruptController::new();
        self.joypad = crate::joypad::Joypad::new();
        self.frame_count = 0;
        self.total_cycles = 0;
        self.instruction_count = 0;
    }

    /// Run one frame of emulation (~16.74ms of Game Boy time).
    /// Returns the number of instructions executed this frame.
    pub fn step_frame(&mut self) -> u32 {
        let mut cycles_elapsed: u32 = 0;
        let mut instructions_this_frame: u32 = 0;

        let cycles_per_frame = if self.memory.is_double_speed() {
            CYCLES_PER_FRAME_DOUBLE
        } else {
            CYCLES_PER_FRAME
        };
        while cycles_elapsed < cycles_per_frame {
            let cycles = {
                let mut bus = MemoryBus::new(
                    &mut self.memory,
                    &mut self.timer,
                    &mut self.joypad,
                    &mut self.apu,
                );
                self.cpu.step(&mut bus, &mut self.interrupts)
            };

            self.timer.tick(cycles, &mut self.memory, &self.interrupts);
            self.ppu.tick(cycles, &mut self.memory, &self.interrupts);
            if self.ppu.took_hblank_step() {
                self.memory.tick_hdma_hblank();
            }
            // In GBC double-speed mode the CPU runs at 8.388 MHz but the APU runs at
            // 4.194 MHz (1×).  Halve the cycles so channel timers advance at the correct
            // rate.  div_counter also increments 2× faster in double-speed, so shift it
            // right by 1 to keep the frame-sequencer bit-12 edge at the correct 512 Hz.
            let (apu_cycles, apu_div) = if self.memory.is_double_speed() {
                (cycles / 2, self.timer.div_counter() >> 1)
            } else {
                (cycles, self.timer.div_counter())
            };
            self.apu.tick(apu_cycles, apu_div);

            cycles_elapsed += cycles;
            instructions_this_frame += 1;
            self.instruction_count += 1;
        }

        self.total_cycles += cycles_elapsed as u64;
        self.frame_count += 1;

        self.memory.tick_rtc();
        self.render_frame();
        instructions_this_frame
    }

    /// Execute a single CPU instruction, ticking timer and PPU.
    /// If a frame boundary is crossed (VBlank entry), renders the frame.
    /// Returns the number of T-cycles consumed.
    pub fn step_single(&mut self) -> u32 {
        let cycles = {
            let mut bus = MemoryBus::new(
                &mut self.memory,
                &mut self.timer,
                &mut self.joypad,
                &mut self.apu,
            );
            self.cpu.step(&mut bus, &mut self.interrupts)
        };

        self.timer.tick(cycles, &mut self.memory, &self.interrupts);
        self.ppu.tick(cycles, &mut self.memory, &self.interrupts);
        if self.ppu.took_hblank_step() {
            self.memory.tick_hdma_hblank();
        }
        let (apu_cycles, apu_div) = if self.memory.is_double_speed() {
            (cycles / 2, self.timer.div_counter() >> 1)
        } else {
            (cycles, self.timer.div_counter())
        };
        self.apu.tick(apu_cycles, apu_div);

        self.total_cycles += cycles as u64;
        self.instruction_count += 1;

        if self.ppu.frame_ready() {
            self.frame_count += 1;
            self.render_frame();
        }

        cycles
    }

    fn render_frame(&mut self) {
        // PPU writes RGBA directly — just copy the completed scanlines into the front buffer.
        self.frame_buffer
            .back_mut()
            .copy_from_slice(self.ppu.get_buffer());
        self.frame_buffer.swap();
    }

    pub fn set_button(&mut self, button: u8, pressed: bool) {
        if let Some(btn) = crate::joypad::Button::from_u8(button) {
            self.joypad.set_button(btn, pressed);
            if pressed {
                self.interrupts.request(Interrupt::Joypad, &mut self.memory);
            }
        }
    }

    pub fn set_camera_image(&mut self, data: &[u8]) {
        self.memory.set_camera_image(data);
    }

    pub fn is_camera_cartridge(&self) -> bool {
        self.memory.get_mbc_type() == crate::memory::MbcType::PocketCamera
    }

    pub fn is_camera_ready(&self) -> bool {
        self.memory.is_camera_image_ready()
    }

    pub fn update_camera_live(&mut self) -> bool {
        if !self.memory.is_camera_capture_dirty() {
            return false;
        }
        self.memory.clear_camera_capture_dirty();

        let sram = self.memory.camera_capture_sram();
        let palette: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];
        let buf = self.camera_live_buffer.back_mut();

        for tile_y in 0..14 {
            for tile_x in 0..16 {
                let tile_offset = (tile_y * 16 + tile_x) * 16;
                for row in 0..8 {
                    let low = sram[tile_offset + row * 2];
                    let high = sram[tile_offset + row * 2 + 1];
                    for col in 0..8 {
                        let bit = 7 - col;
                        let color_idx = ((high >> bit) & 1) << 1 | ((low >> bit) & 1);
                        let gray = palette[color_idx as usize];
                        let px = tile_x * 8 + col;
                        let py = tile_y * 8 + row;
                        let i = (py * 128 + px) * 4;
                        buf[i] = gray;
                        buf[i + 1] = gray;
                        buf[i + 2] = gray;
                        buf[i + 3] = 255;
                    }
                }
            }
        }
        self.camera_live_buffer.swap();
        true
    }

    pub fn decode_camera_photo(&self, slot: u8) -> Vec<u8> {
        self.memory.decode_camera_photo(slot)
    }

    #[cfg_attr(not(feature = "ios"), allow(dead_code))] // ios: gb_encode_camera_photo
    pub fn encode_camera_photo(&mut self, slot: u8, rgba: &[u8]) -> bool {
        self.memory.encode_camera_photo(slot, rgba)
    }

    #[cfg_attr(not(feature = "ios"), allow(dead_code))] // ios: gb_clear_camera_photo_slot
    pub fn clear_camera_photo_slot(&mut self, slot: u8) {
        self.memory.clear_camera_photo_slot(slot)
    }

    #[cfg_attr(not(feature = "ios"), allow(dead_code))] // ios: gb_camera_photo_count
    pub fn camera_photo_count(&self) -> u8 {
        self.memory.camera_photo_count()
    }
}

// ── Native API ───────────────────────────────────────────────────────────────
//
// Public methods used by native binaries
//

impl GameBoyCore {
    // ── Memory access ────────────────────────────────────────────────────────

    pub fn read_byte(&self, addr: u16) -> u8 {
        self.memory.read(addr)
    }

    pub fn write_byte(&mut self, addr: u16, value: u8) {
        self.memory.write(addr, value);
    }

    pub fn read_io(&self, offset: u8) -> u8 {
        self.memory.read_io_direct(offset)
    }

    pub fn write_io(&mut self, offset: u8, value: u8) {
        self.memory.write_io_direct(offset, value);
    }

    pub fn get_cartridge_ram(&self) -> &[u8] {
        self.memory.get_cartridge_ram()
    }

    pub fn load_cartridge_ram(&mut self, data: &[u8]) {
        self.memory.load_cartridge_ram(data);
    }

    // ── Sample-accurate stepping ─────────────────────────────────────────────

    /// Run until at least `target` stereo sample pairs are in the APU buffer.
    /// Returns the number of pairs actually generated (may exceed `target` by
    /// up to one instruction's worth of samples due to instruction granularity).
    ///
    /// If the APU is powered off or the CPU is permanently halted, the loop
    /// exits after a cycle budget of `target × 200` cycles to prevent hangs.
    pub fn step_samples(&mut self, target: usize) -> usize {
        let before = self.apu.sample_buf.len() / 2;
        // ~95 cycles per sample at 4.19 MHz / 44100 Hz; 200× is a generous budget.
        let max_cycles = self.total_cycles + (target as u64) * 200;
        while (self.apu.sample_buf.len() / 2) - before < target {
            if self.total_cycles >= max_cycles {
                break;
            }
            self.step_single();
        }
        (self.apu.sample_buf.len() / 2) - before
    }

    // ── Serial port injection ────────────────────────────────────────────────

    /// Simulate an external device sending one byte to the GB serial port.
    /// Places `byte` in SB, configures SC for external-clock mode, and fires
    /// the serial interrupt (IF bit 3).
    pub fn serial_inject(&mut self, byte: u8) {
        self.memory.write_io_direct(0x01, byte); // SB
        let sc = self.memory.read_io_direct(0x02) & 0x7E; // SC: external clock
        self.memory.write_io_direct(0x02, sc);
        self.interrupts.request(Interrupt::Serial, &mut self.memory); // IF bit 3
    }

    /// Return and remove the oldest byte from the serial output buffer, or
    /// `None` if the GB has not transmitted anything since the last call.
    pub fn serial_take_output(&mut self) -> Option<u8> {
        self.memory.serial_take_output()
    }

    // ── APU state ────────────────────────────────────────────────────────────

    pub fn apu_powered(&self) -> bool {
        self.apu.powered()
    }
    pub fn apu_sample_len(&self) -> usize {
        self.apu.sample_buf.len()
    }
    pub fn apu_sample_buf(&self) -> &[f32] {
        &self.apu.sample_buf
    }
    pub fn apu_clear_samples(&mut self) {
        self.apu.clear_samples();
    }

    pub fn apu_ch1_enabled(&self) -> bool {
        self.apu.ch1.enabled
    }
    pub fn apu_ch1_freq_hz(&self) -> f32 {
        self.apu.ch1.freq_hz()
    }

    pub fn apu_ch2_enabled(&self) -> bool {
        self.apu.ch2.enabled
    }
    pub fn apu_ch2_freq_hz(&self) -> f32 {
        self.apu.ch2.freq_hz()
    }

    pub fn apu_ch3_enabled(&self) -> bool {
        self.apu.ch3.enabled
    }
    pub fn apu_ch3_freq_hz(&self) -> f32 {
        self.apu.ch3.freq_hz()
    }

    pub fn apu_ch4_enabled(&self) -> bool {
        self.apu.ch4.enabled
    }

    // ── Counters ─────────────────────────────────────────────────────────────

    pub fn total_cycles(&self) -> u64 {
        self.total_cycles
    }
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }
}

// ── Snapshot / Restore ────────────────────────────────────────────────────────

const SNAP_MAGIC: &[u8] = b"GBSNAP1";
const SNAP_VERSION: u8 = 1;

impl GameBoyCore {
    /// Serialize the full emulator state to a byte buffer.
    ///
    /// Excludes the ROM binary (the caller holds it) and the audio output
    /// ring buffer, so `audio_sample_buffer_ptr()` remains valid and queued
    /// samples play through without a click after `restore()`.
    ///
    /// Typical sizes: ~50 KB (no cart RAM) · ~82 KB (32 KB LSDJ cart RAM).
    pub fn snapshot(&self) -> Vec<u8> {
        let mut w = SnapWriter::new();
        w.bytes(SNAP_MAGIC);
        w.u8(SNAP_VERSION);
        self.cpu.snapshot(&mut w);
        self.timer.snapshot(&mut w);
        self.ppu.snapshot(&mut w);
        self.apu.snapshot(&mut w);
        self.joypad.snapshot(&mut w);
        self.memory.snapshot(&mut w);
        w.u32(self.frame_count);
        w.u64(self.total_cycles);
        w.u64(self.instruction_count);
        w.into_vec()
    }

    /// Restore emulator state from a buffer produced by `snapshot()`.
    ///
    /// The audio output ring buffer is preserved — no audible click.
    /// Returns an error if the buffer is truncated or version-incompatible.
    pub fn restore(&mut self, data: &[u8]) -> Result<(), &'static str> {
        let mut r = SnapReader::new(data);
        let magic = r.bytes(SNAP_MAGIC.len())?;
        if magic != SNAP_MAGIC {
            return Err("invalid snapshot magic");
        }
        if r.u8()? != SNAP_VERSION {
            return Err("unsupported snapshot version");
        }
        self.cpu.restore_from(&mut r)?;
        self.timer.restore_from(&mut r)?;
        self.ppu.restore_from(&mut r)?;
        self.apu.restore_from(&mut r)?;
        self.joypad.restore_from(&mut r)?;
        self.memory.restore_from(&mut r)?;
        self.frame_count = r.u32()?;
        self.total_cycles = r.u64()?;
        self.instruction_count = r.u64()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 32 KB NoMBC ROM: NOP at 0x0100, then JR -2 (infinite loop).
    fn minimal_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x0100] = 0x00; // NOP
        rom[0x0101] = 0x18; // JR
        rom[0x0102] = 0xFE; // -2  →  loops forever at 0x0101
        rom
    }

    /// 64 KB MBC1 ROM (4 banks).
    /// Bank 1 @ 0x4000 = 0xAA, bank 2 @ 0x8000 = 0xBB.
    fn mbc1_multibank_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x10000];
        rom[0x0147] = 0x01; // MBC1
        rom[0x0100] = 0x18; // JR -2
        rom[0x0101] = 0xFE;
        rom[0x4000] = 0xAA; // bank 1 sentinel
        rom[0x8000] = 0xBB; // bank 2 sentinel
        rom
    }

    /// 32 KB MBC5+RAM ROM with 8 KB SRAM.
    fn mbc5_rom_with_ram() -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x0147] = 0x1B; // MBC5+RAM+BATTERY
        rom[0x0149] = 0x02; // 8 KB RAM
        rom[0x0100] = 0x18; // JR -2
        rom[0x0101] = 0xFE;
        rom
    }

    #[test]
    fn snapshot_identity() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();

        for _ in 0..5 { core.step_frame(); }
        let snap_a = core.snapshot();

        for _ in 0..5 { core.step_frame(); }
        core.restore(&snap_a).unwrap();
        let snap_b = core.snapshot();

        assert_eq!(snap_a, snap_b, "snapshot after restore must be byte-identical");
    }

    #[test]
    fn snapshot_counters_preserved() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();

        for _ in 0..4 { core.step_frame(); }
        let frame_count_before = core.frame_count;
        let total_cycles_before = core.total_cycles;

        let snap = core.snapshot();
        for _ in 0..10 { core.step_frame(); }

        core.restore(&snap).unwrap();
        assert_eq!(core.frame_count, frame_count_before);
        assert_eq!(core.total_cycles, total_cycles_before);
    }

    #[test]
    fn snapshot_audio_ring_buffer_preserved() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();

        // Run until the APU has produced samples.
        core.step_samples(128);
        let sample_count = core.apu.sample_buf.len();
        assert!(sample_count > 0, "expected audio samples before snapshot");
        let sample_snapshot: Vec<f32> = core.apu.sample_buf.clone();

        let snap = core.snapshot();
        core.restore(&snap).unwrap();

        // Ring buffer must be intact: same length, same values.
        assert_eq!(core.apu.sample_buf.len(), sample_count,
            "restore() must not touch the audio ring buffer");
        assert_eq!(core.apu.sample_buf, sample_snapshot);
    }

    #[test]
    fn snapshot_cart_ram_preserved() {
        let mut core = GameBoyCore::new();
        core.load_rom(&mbc5_rom_with_ram(), false).unwrap();

        // Directly load known data into cart RAM (bypasses MBC enable logic).
        let original: Vec<u8> = (0..8).collect();
        core.memory.load_cartridge_ram(&original);

        let snap = core.snapshot();

        // Overwrite cart RAM after snapshotting.
        let zeroed = vec![0u8; 8];
        core.memory.load_cartridge_ram(&zeroed);
        assert_eq!(core.get_cartridge_ram()[0], 0);

        core.restore(&snap).unwrap();
        assert_eq!(&core.get_cartridge_ram()[..8], &original[..]);
    }

    #[test]
    fn snapshot_mbc_banking_preserved() {
        let mut core = GameBoyCore::new();
        core.load_rom(&mbc1_multibank_rom(), false).unwrap();

        // Switch to ROM bank 2 and verify the sentinel.
        core.write_byte(0x2000, 0x02);
        assert_eq!(core.read_byte(0x4000), 0xBB, "bank 2 sentinel before snapshot");

        let snap = core.snapshot();

        // Switch away from bank 2.
        core.write_byte(0x2000, 0x01);
        assert_eq!(core.read_byte(0x4000), 0xAA, "bank 1 after switching away");

        core.restore(&snap).unwrap();
        assert_eq!(core.read_byte(0x4000), 0xBB, "bank 2 sentinel restored");
    }

    #[test]
    fn snapshot_invalid_magic() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();
        let mut bad = core.snapshot();
        bad[0] = b'X'; // corrupt magic
        assert!(core.restore(&bad).is_err());
    }

    #[test]
    fn snapshot_empty_buffer() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();
        assert!(core.restore(&[]).is_err());
    }

    #[test]
    fn snapshot_truncated() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();
        let snap = core.snapshot();
        let truncated = &snap[..snap.len() / 2];
        assert!(core.restore(truncated).is_err());
    }

    #[test]
    fn snapshot_wrong_version() {
        let mut core = GameBoyCore::new();
        core.load_rom(&minimal_rom(), false).unwrap();
        let mut snap = core.snapshot();
        snap[7] = 0xFF; // version byte is at index 7 (after 7-byte magic)
        assert!(core.restore(&snap).is_err());
    }
}
