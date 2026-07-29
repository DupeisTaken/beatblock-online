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
    format = 0;
    memcpy(header + FRAME_PIXEL_FORMAT_OFFSET, &format, sizeof(format));
    assert(!frame_header_has_supported_pixels(header));
    return 0;
}
