//! C-compatible FFI layer for iOS integration.
//!
//! This module provides extern "C" functions that can be called from Swift
//! via a bridging header. All emulation logic lives in [`GameBoyCore`]; this
//! module is a thin adapter between C calling conventions and the core.

use std::ffi::c_void;
use std::ptr;
use std::slice;

use crate::core::GameBoyCore;

/// Opaque GameBoy emulator handle for FFI.
struct GameBoyHandle {
    core: GameBoyCore,
}

// ============================================================================
// C FFI Functions
// ============================================================================

/// Create a new GameBoy emulator instance.
/// Returns an opaque pointer that must be freed with `gb_destroy`.
#[unsafe(no_mangle)]
pub extern "C" fn gb_create() -> *mut c_void {
    let handle = Box::new(GameBoyHandle {
        core: GameBoyCore::new(),
    });
    Box::into_raw(handle) as *mut c_void
}

/// Destroy a GameBoy emulator instance.
/// The pointer must have been created by `gb_create`.
#[unsafe(no_mangle)]
pub extern "C" fn gb_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        unsafe {
            let _ = Box::from_raw(handle as *mut GameBoyHandle);
        }
    }
}

/// Load a ROM into the emulator.
/// Pass cgb_mode=true to run in Game Boy Color mode (enables colour palettes, VRAM banking, etc.).
/// Pass cgb_mode=false for standard DMG mode (existing behaviour).
/// Returns true on success, false on failure.
#[unsafe(no_mangle)]
pub extern "C" fn gb_load_rom(
    handle: *mut c_void,
    data: *const u8,
    len: usize,
    cgb_mode: bool,
) -> bool {
    if handle.is_null() || data.is_null() || len == 0 {
        return false;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        let rom_data = slice::from_raw_parts(data, len);
        gb.core.load_rom(rom_data, cgb_mode).is_ok()
    }
}

/// Power-cycle: reset CPU, PPU, APU, timer, and MBC banking state.
/// Cartridge SRAM is cleared; call `gb_load_cartridge_ram` to restore it.
#[unsafe(no_mangle)]
pub extern "C" fn gb_reset(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.reset();
    }
}

/// Run one frame of emulation (~16.74ms of Game Boy time).
#[unsafe(no_mangle)]
pub extern "C" fn gb_step_frame(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.step_frame();
    }
}

/// Run until at least `n` stereo sample pairs have been generated.
/// Returns the number of pairs actually produced (may exceed `n` by up to one
/// instruction's worth due to granularity).
#[unsafe(no_mangle)]
pub extern "C" fn gb_step_samples(handle: *mut c_void, n: usize) -> usize {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.step_samples(n)
    }
}

/// Simulate an external device sending one byte on the serial link.
/// Places `byte` in SB, sets SC to external-clock mode, fires the serial interrupt.
#[unsafe(no_mangle)]
pub extern "C" fn gb_serial_inject(handle: *mut c_void, byte: u8) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.serial_inject(byte);
    }
}

/// Return and consume the oldest byte from the serial output buffer.
/// Writes the byte to `*out` and returns true if one was available; returns
/// false (and leaves `*out` unchanged) if the buffer is empty.
#[unsafe(no_mangle)]
pub extern "C" fn gb_serial_take_output(handle: *mut c_void, out: *mut u8) -> bool {
    if handle.is_null() || out.is_null() {
        return false;
    }
    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        if let Some(byte) = gb.core.serial_take_output() {
            *out = byte;
            true
        } else {
            false
        }
    }
}

/// Get a pointer to the frame buffer (160x144 RGBA pixels).
/// The buffer is owned by the emulator and valid until the next call or destruction.
/// Returns NULL if handle is invalid.
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_frame_buffer(handle: *const c_void) -> *const u8 {
    if handle.is_null() {
        return ptr::null();
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.frame_buffer.front().as_ptr()
    }
}

/// Get the frame buffer size in bytes (always 160 * 144 * 4 = 92160).
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_frame_buffer_size() -> usize {
    160 * 144 * 4
}

/// Get the screen width in pixels.
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_screen_width() -> u32 {
    160
}

/// Get the screen height in pixels.
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_screen_height() -> u32 {
    144
}

/// Set button state.
/// Button values: 0=A, 1=B, 2=Select, 3=Start, 4=Right, 5=Left, 6=Up, 7=Down
#[unsafe(no_mangle)]
pub extern "C" fn gb_set_button(handle: *mut c_void, button: u8, pressed: bool) {
    if handle.is_null() || button > 7 {
        return;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.set_button(button, pressed);
    }
}

/// Set camera image data for Game Boy Camera emulation.
/// Expects 128x112 pixels as 8-bit grayscale (0=black, 255=white).
#[unsafe(no_mangle)]
pub extern "C" fn gb_set_camera_image(handle: *mut c_void, data: *const u8, len: usize) {
    if handle.is_null() || data.is_null() {
        return;
    }

    let expected_len = 128 * 112;
    if len < expected_len {
        return;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        let image_data = slice::from_raw_parts(data, expected_len);
        gb.core.set_camera_image(image_data);
    }
}

/// Check if the loaded ROM is a Game Boy Camera cartridge.
#[unsafe(no_mangle)]
pub extern "C" fn gb_is_camera_cartridge(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.is_camera_cartridge()
    }
}

/// Check if the camera has image data ready for capture.
#[unsafe(no_mangle)]
pub extern "C" fn gb_is_camera_ready(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.is_camera_ready()
    }
}

/// Update the camera live view buffer from the active capture SRAM.
/// Returns true if the buffer was updated (i.e. capture data changed since last call).
#[unsafe(no_mangle)]
pub extern "C" fn gb_update_camera_live(handle: *mut c_void) -> bool {
    if handle.is_null() {
        return false;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.update_camera_live()
    }
}

/// Get a pointer to the camera live view buffer (128x112 RGBA pixels).
/// The buffer is owned by the emulator and valid until the next call or destruction.
/// Returns NULL if handle is invalid.
#[unsafe(no_mangle)]
pub extern "C" fn gb_camera_live_ptr(handle: *const c_void) -> *const u8 {
    if handle.is_null() {
        return ptr::null();
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.camera_live_buffer.front().as_ptr()
    }
}

/// Get the camera live view buffer size in bytes (always 128 * 112 * 4 = 57344).
#[unsafe(no_mangle)]
pub extern "C" fn gb_camera_live_len() -> usize {
    128 * 112 * 4
}

/// Decode a GB Camera saved photo slot to RGBA pixel data.
/// Slots 1-30 = saved photos. Writes up to `buffer_len` bytes into `buffer`.
/// Returns the number of bytes written, or 0 if the slot is empty/unoccupied.
#[unsafe(no_mangle)]
pub extern "C" fn gb_decode_camera_photo(
    handle: *const c_void,
    slot: u8,
    buffer: *mut u8,
    buffer_len: usize,
) -> usize {
    if handle.is_null() || buffer.is_null() {
        return 0;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        let rgba = gb.core.decode_camera_photo(slot);
        if rgba.is_empty() {
            return 0;
        }

        let copy_len = rgba.len().min(buffer_len);
        if copy_len > 0 {
            ptr::copy_nonoverlapping(rgba.as_ptr(), buffer, copy_len);
        }
        copy_len
    }
}

/// Get the current frame count.
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_frame_count(handle: *const c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.frame_count
    }
}

/// Return the size of the cartridge SRAM in bytes (0 if no RAM).
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_cartridge_ram_len(handle: *const c_void) -> usize {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.memory.get_cartridge_ram().len()
    }
}

/// Copy cartridge SRAM to `buffer`. Returns bytes copied, or 0 on error.
#[unsafe(no_mangle)]
pub extern "C" fn gb_get_cartridge_ram(
    handle: *const c_void,
    buffer: *mut u8,
    buffer_len: usize,
) -> usize {
    if handle.is_null() || buffer.is_null() {
        return 0;
    }
    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        let ram = gb.core.memory.get_cartridge_ram();
        let n = ram.len().min(buffer_len);
        if n > 0 {
            ptr::copy_nonoverlapping(ram.as_ptr(), buffer, n);
        }
        n
    }
}

/// Load cartridge SRAM from `data`. Returns true on success.
#[unsafe(no_mangle)]
pub extern "C" fn gb_load_cartridge_ram(
    handle: *mut c_void,
    data: *const u8,
    len: usize,
) -> bool {
    if handle.is_null() || data.is_null() || len == 0 {
        return false;
    }
    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        let save_data = slice::from_raw_parts(data, len);
        gb.core.memory.load_cartridge_ram(save_data);
        true
    }
}

/// Get the current camera contrast level (0-15, or -1 if unknown).
#[unsafe(no_mangle)]
pub extern "C" fn gb_camera_contrast(handle: *const c_void) -> i32 {
    if handle.is_null() {
        return -1;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.memory.camera_contrast()
    }
}

/// Set or clear the camera exposure override.
/// When `exposure` is 0-65535, that value is used instead of the ROM's.
/// When `exposure` is -1, the override is cleared and the ROM controls exposure.
#[unsafe(no_mangle)]
pub extern "C" fn gb_set_camera_exposure(handle: *mut c_void, exposure: i32) {
    if handle.is_null() {
        return;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        if exposure < 0 {
            gb.core.memory.set_camera_exposure_override(None);
        } else {
            gb.core
                .memory
                .set_camera_exposure_override(Some(exposure as u16));
        }
    }
}

/// Encode RGBA pixel data into a GB Camera SRAM slot.
/// Slots 1-30 = saved photos. `rgba` must point to 128*112*4 bytes.
/// Returns true on success, false on invalid slot or bad data.
#[unsafe(no_mangle)]
pub extern "C" fn gb_encode_camera_photo(
    handle: *mut c_void,
    slot: u8,
    rgba: *const u8,
    len: usize,
) -> bool {
    if handle.is_null() || rgba.is_null() {
        return false;
    }

    let expected = 128 * 112 * 4;
    if len != expected {
        return false;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        let data = slice::from_raw_parts(rgba, len);
        gb.core.encode_camera_photo(slot, data)
    }
}

/// Clear a GB Camera SRAM slot (zero out tile data).
/// Slots 1-30 = saved photos.
#[unsafe(no_mangle)]
pub extern "C" fn gb_clear_camera_photo_slot(handle: *mut c_void, slot: u8) {
    if handle.is_null() {
        return;
    }

    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.clear_camera_photo_slot(slot);
    }
}

/// Get the number of occupied photo slots (0-30) by scanning the SRAM state vector.
#[unsafe(no_mangle)]
pub extern "C" fn gb_camera_photo_count(handle: *const c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.camera_photo_count()
    }
}

/// Read a byte from any memory address (for HRAM polling etc.).
#[unsafe(no_mangle)]
pub extern "C" fn gb_read_memory(handle: *const c_void, addr: u16) -> u8 {
    if handle.is_null() {
        return 0;
    }

    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.memory.read(addr)
    }
}

// Button constants for Swift
pub const GB_BUTTON_A: u8 = crate::joypad::Button::A as u8;
pub const GB_BUTTON_B: u8 = crate::joypad::Button::B as u8;
pub const GB_BUTTON_SELECT: u8 = crate::joypad::Button::Select as u8;
pub const GB_BUTTON_START: u8 = crate::joypad::Button::Start as u8;
pub const GB_BUTTON_RIGHT: u8 = crate::joypad::Button::Right as u8;
pub const GB_BUTTON_LEFT: u8 = crate::joypad::Button::Left as u8;
pub const GB_BUTTON_UP: u8 = crate::joypad::Button::Up as u8;
pub const GB_BUTTON_DOWN: u8 = crate::joypad::Button::Down as u8;

// Screen dimensions
pub const GB_SCREEN_WIDTH: u32 = 160;
pub const GB_SCREEN_HEIGHT: u32 = 144;

// Camera dimensions
pub const GB_CAMERA_WIDTH: u32 = 128;
pub const GB_CAMERA_HEIGHT: u32 = 112;

// ── Audio / APU ──────────────────────────────────────────────────────────────

/// Target audio sample rate in Hz (44 100).
#[unsafe(no_mangle)]
pub extern "C" fn gb_audio_sample_rate() -> u32 {
    crate::apu::SAMPLE_RATE
}

/// Pointer to the current interleaved stereo f32 sample buffer (L, R, L, R, …).
/// Valid until the next `gb_step_frame` call.  Returns NULL for an invalid handle.
#[unsafe(no_mangle)]
pub extern "C" fn gb_audio_sample_ptr(handle: *const c_void) -> *const f32 {
    if handle.is_null() {
        return ptr::null();
    }
    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.apu.sample_buf.as_ptr()
    }
}

/// Number of f32 values in the sample buffer (stereo: length / 2 = sample count).
#[unsafe(no_mangle)]
pub extern "C" fn gb_audio_sample_len(handle: *const c_void) -> usize {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.apu.sample_buf.len()
    }
}

/// Clear the audio sample buffer after the host has consumed it.
/// Call this once per frame after reading the samples.
#[unsafe(no_mangle)]
pub extern "C" fn gb_audio_clear_samples(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let gb = &mut *(handle as *mut GameBoyHandle);
        gb.core.apu.clear_samples();
    }
}

/// Returns 1 if the APU is powered on (NR52 bit 7), 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_powered(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        gb.core.apu.powered()
    }
}

// ── Per-channel debug info ────────────────────────────────────────────────────

/// CH1 frequency register value (0–2047).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch1_freq_reg(handle: *const c_void) -> u16 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch1.frequency() }
}

/// CH1 envelope volume (0–15).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch1_volume(handle: *const c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch1.env_volume }
}

/// CH1 enabled flag.
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch1_enabled(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch1.enabled }
}

/// CH2 frequency register value (0–2047).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch2_freq_reg(handle: *const c_void) -> u16 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch2.frequency() }
}

/// CH2 envelope volume (0–15).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch2_volume(handle: *const c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch2.env_volume }
}

/// CH2 enabled flag.
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch2_enabled(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch2.enabled }
}

/// CH3 frequency register value (0–2047).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch3_freq_reg(handle: *const c_void) -> u16 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch3.frequency() }
}

/// CH3 volume code (0=mute, 1=100%, 2=50%, 3=25%).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch3_vol_code(handle: *const c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        (*(handle as *const GameBoyHandle))
            .core
            .apu
            .ch3
            .volume_code()
    }
}

/// CH3 enabled flag.
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch3_enabled(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch3.enabled }
}

/// Copy the 16-byte wave RAM into `buf` (caller must provide ≥16 bytes).
/// Returns number of bytes written.
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch3_wave_ram(handle: *const c_void, buf: *mut u8, len: usize) -> usize {
    if handle.is_null() || buf.is_null() || len < 16 {
        return 0;
    }
    unsafe {
        let gb = &*(handle as *const GameBoyHandle);
        let wave = &gb.core.apu.ch3.wave_ram;
        std::ptr::copy_nonoverlapping(wave.as_ptr(), buf, 16);
    }
    16
}

/// CH4 envelope volume (0–15).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch4_volume(handle: *const c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch4.env_volume }
}

/// CH4 enabled flag.
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch4_enabled(handle: *const c_void) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch4.enabled }
}

/// CH4 NR43 register (clock config: shift | width | divider).
#[unsafe(no_mangle)]
pub extern "C" fn gb_apu_ch4_nr43(handle: *const c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe { (*(handle as *const GameBoyHandle)).core.apu.ch4.nr43 }
}
