#include <assert.h>
#include <stdint.h>
#include <string.h>

#define BBT_AUDIO_TARGET_TEST
#include "../src/plugin.c"

int main(void)
{
    char window[128];

    build_audio_window("A", window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Renderer A:SDL_app:Beatblock.exe") == 0);

    build_audio_window("d", window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Renderer D:SDL_app:Beatblock.exe") == 0);

    build_audio_window("AUTOPLAY", window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Autoplay:SDL_app:Beatblock.exe") == 0);

    build_audio_window("?", window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Renderer A:SDL_app:Beatblock.exe") == 0);

    uint8_t header[HEADER_SIZE] = {0};
    memcpy(header, FRAME_MAGIC, 8);
    uint32_t version = FRAME_VERSION;
    uint32_t format = FRAME_PIXEL_FORMAT_RGBA8_DISPLAY_SRGB;
    memcpy(header + 8, &version, sizeof(version));
    memcpy(header + FRAME_PIXEL_FORMAT_OFFSET, &format, sizeof(format));
    assert(frame_header_has_supported_pixels(header));

    uint32_t frame_count = 3;
    uint64_t sequence = 1;
    memcpy(header + 24, &frame_count, sizeof(frame_count));
    memcpy(header + 32, &sequence, sizeof(sequence));
    size_t slot_offset = frame_slot_sequence_offset(sequence, frame_count);
    memcpy(header + slot_offset, &sequence, sizeof(sequence));
    assert(frame_header_has_committed_frame(header, frame_count, sequence));

    // An in-progress overwrite invalidates the slot before changing any pixels.
    uint64_t in_progress = 0;
    memcpy(header + slot_offset, &in_progress, sizeof(in_progress));
    assert(!frame_header_has_committed_frame(header, frame_count, sequence));

    // Sequence N+3 reuses N's modulo slot. It must not validate while the
    // global commit still advertises N, even if the new copy finishes quickly.
    uint64_t reused_sequence = sequence + frame_count;
    memcpy(header + slot_offset, &reused_sequence, sizeof(reused_sequence));
    assert(!frame_header_has_committed_frame(header, frame_count, sequence));
    memcpy(header + 32, &reused_sequence, sizeof(reused_sequence));
    assert(frame_header_has_committed_frame(header, frame_count, reused_sequence));

    uint32_t old_version = FRAME_VERSION - 1;
    memcpy(header + 8, &old_version, sizeof(old_version));
    assert(!frame_header_has_supported_pixels(header));
    memcpy(header + 8, &version, sizeof(version));
    format = 0;
    memcpy(header + FRAME_PIXEL_FORMAT_OFFSET, &format, sizeof(format));
    assert(!frame_header_has_supported_pixels(header));
    return 0;
}
