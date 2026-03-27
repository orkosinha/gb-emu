#ifndef GB_EMU_H
#define GB_EMU_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

// Button constants
#define GB_BUTTON_A 0
#define GB_BUTTON_B 1
#define GB_BUTTON_SELECT 2
#define GB_BUTTON_START 3
#define GB_BUTTON_RIGHT 4
#define GB_BUTTON_LEFT 5
#define GB_BUTTON_UP 6
#define GB_BUTTON_DOWN 7

// Screen dimensions
#define GB_SCREEN_WIDTH 160
#define GB_SCREEN_HEIGHT 144

// Camera dimensions
#define GB_CAMERA_WIDTH 128
#define GB_CAMERA_HEIGHT 112

// Opaque handle to GameBoy emulator instance
typedef void* GBHandle;

// Lifecycle
GBHandle gb_create(void);
void gb_destroy(GBHandle handle);

// ROM loading
bool gb_load_rom(GBHandle handle, const uint8_t* data, size_t len);

// Emulation
void gb_step_frame(GBHandle handle);
uint32_t gb_get_frame_count(GBHandle handle);

// Frame buffer
const uint8_t* gb_get_frame_buffer(GBHandle handle);
size_t gb_get_frame_buffer_size(void);
uint32_t gb_get_screen_width(void);
uint32_t gb_get_screen_height(void);

// Input
void gb_set_button(GBHandle handle, uint8_t button, bool pressed);

// Camera
void gb_set_camera_image(GBHandle handle, const uint8_t* data, size_t len);
bool gb_is_camera_cartridge(GBHandle handle);
bool gb_is_camera_ready(GBHandle handle);
bool gb_update_camera_live(GBHandle handle);
const uint8_t* gb_camera_live_ptr(GBHandle handle);
size_t gb_camera_live_len(void);
size_t gb_decode_camera_photo(GBHandle handle, uint8_t slot, uint8_t* buffer, size_t buffer_len);
int32_t gb_camera_contrast(GBHandle handle);
void gb_set_camera_exposure(GBHandle handle, int32_t exposure);
bool gb_encode_camera_photo(GBHandle handle, uint8_t slot, const uint8_t* rgba, size_t len);
void gb_clear_camera_photo_slot(GBHandle handle, uint8_t slot);
uint8_t gb_camera_photo_count(GBHandle handle);

// Memory
uint8_t gb_read_memory(GBHandle handle, uint16_t addr);

// Save data
size_t gb_get_save_size(GBHandle handle);
size_t gb_get_save_data(GBHandle handle, uint8_t* buffer, size_t buffer_len);
bool gb_load_save_data(GBHandle handle, const uint8_t* data, size_t len);

// Audio / APU
// Samples are interleaved stereo f32: [L0, R0, L1, R1, ...]
// Call gb_audio_clear_samples() once per frame after consuming the buffer.
uint32_t gb_audio_sample_rate(void);
const float* gb_audio_sample_ptr(GBHandle handle);
size_t gb_audio_sample_len(GBHandle handle);
void gb_audio_clear_samples(GBHandle handle);
bool gb_apu_powered(GBHandle handle);

// APU channel debug (for LSDJ-style visualiser)
uint16_t gb_apu_ch1_freq_reg(GBHandle handle);
uint8_t  gb_apu_ch1_volume(GBHandle handle);
bool     gb_apu_ch1_enabled(GBHandle handle);
uint16_t gb_apu_ch2_freq_reg(GBHandle handle);
uint8_t  gb_apu_ch2_volume(GBHandle handle);
bool     gb_apu_ch2_enabled(GBHandle handle);
uint16_t gb_apu_ch3_freq_reg(GBHandle handle);
uint8_t  gb_apu_ch3_vol_code(GBHandle handle);
bool     gb_apu_ch3_enabled(GBHandle handle);
size_t   gb_apu_ch3_wave_ram(GBHandle handle, uint8_t* buf, size_t len);
uint8_t  gb_apu_ch4_volume(GBHandle handle);
bool     gb_apu_ch4_enabled(GBHandle handle);
uint8_t  gb_apu_ch4_nr43(GBHandle handle);

#endif /* GB_EMU_H */
