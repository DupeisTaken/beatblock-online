#include <assert.h>
#include <string.h>

#define BBT_AUDIO_TARGET_TEST
#include "../src/plugin.c"

int main(void)
{
    char window[128];

    build_audio_window('A', window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Renderer A:SDL_app:Beatblock.exe") == 0);

    build_audio_window('d', window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Renderer D:SDL_app:Beatblock.exe") == 0);

    build_audio_window('?', window, sizeof(window));
    assert(strcmp(window, "Beatblock Online Renderer A:SDL_app:Beatblock.exe") == 0);
    return 0;
}
