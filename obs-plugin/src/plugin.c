#include <obs-module.h>
#include <graphics/graphics.h>
#include <util/platform.h>
#include <windows.h>
#include <stdint.h>
#include <stdio.h>

OBS_DECLARE_MODULE()
OBS_MODULE_USE_DEFAULT_LOCALE("beatblock-together-obs", "en-US")

#define HEADER_SIZE 64
#define FRAME_MAGIC "BBTFRAME"

struct bbt_video {
    obs_source_t *source;
    gs_texture_t *texture;
    char slot;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint64_t sequence;
    uint8_t *pixels;
    size_t pixel_capacity;
    uint64_t last_frame_ns;
    wchar_t path[MAX_PATH * 2];
};

static void clear_stale_texture(struct bbt_video *ctx)
{
    if (!ctx->texture || os_gettime_ns() - ctx->last_frame_ns < 1500000000ULL)
        return;
    obs_enter_graphics();
    gs_texture_destroy(ctx->texture);
    ctx->texture = NULL;
    obs_leave_graphics();
    ctx->width = 0;
    ctx->height = 0;
    ctx->sequence = 0;
}

static const char *video_name(void *unused)
{
    UNUSED_PARAMETER(unused);
    return obs_module_text("PlayerStream");
}

static void build_path(struct bbt_video *ctx)
{
    wchar_t root[MAX_PATH] = {0};
    DWORD length = GetEnvironmentVariableW(L"LOCALAPPDATA", root, MAX_PATH);
    if (!length)
        wcscpy_s(root, MAX_PATH, L".");
    _snwprintf_s(ctx->path, MAX_PATH * 2, _TRUNCATE,
        L"%s\\BeatblockTogether\\BeatblockTogether\\data\\render-streams\\stream-%c.bbtframe",
        root, ctx->slot);
}

static void video_update(void *data, obs_data_t *settings)
{
    struct bbt_video *ctx = data;
    const char *slot = obs_data_get_string(settings, "slot");
    ctx->slot = slot && *slot ? slot[0] : 'A';
    if (ctx->slot >= 'a' && ctx->slot <= 'd')
        ctx->slot -= ('a' - 'A');
    build_path(ctx);
}

static void *video_create(obs_data_t *settings, obs_source_t *source)
{
    struct bbt_video *ctx = bzalloc(sizeof(*ctx));
    ctx->source = source;
    ctx->slot = 'A';
    video_update(ctx, settings);
    return ctx;
}

static void video_destroy(void *data)
{
    struct bbt_video *ctx = data;
    obs_enter_graphics();
    if (ctx->texture)
        gs_texture_destroy(ctx->texture);
    obs_leave_graphics();
    bfree(ctx->pixels);
    bfree(ctx);
}

static void video_defaults(obs_data_t *settings)
{
    obs_data_set_default_string(settings, "slot", "A");
}

static obs_properties_t *video_properties(void *data)
{
    UNUSED_PARAMETER(data);
    obs_properties_t *props = obs_properties_create();
    obs_property_t *slot = obs_properties_add_list(props, "slot", obs_module_text("StreamSlot"),
        OBS_COMBO_TYPE_LIST, OBS_COMBO_FORMAT_STRING);
    obs_property_list_add_string(slot, "Stream A", "A");
    obs_property_list_add_string(slot, "Stream B", "B");
    obs_property_list_add_string(slot, "Stream C", "C");
    obs_property_list_add_string(slot, "Stream D", "D");
    obs_properties_add_text(props, "status", obs_module_text("ManagerHint"), OBS_TEXT_INFO);
    return props;
}

static void video_tick(void *data, float seconds)
{
    UNUSED_PARAMETER(seconds);
    struct bbt_video *ctx = data;
    FILE *file = NULL;
    if (_wfopen_s(&file, ctx->path, L"rb") || !file) {
        clear_stale_texture(ctx);
        return;
    }

    uint8_t header[HEADER_SIZE];
    if (fread(header, 1, sizeof(header), file) != sizeof(header) ||
        memcmp(header, FRAME_MAGIC, 8) != 0) {
        fclose(file);
        clear_stale_texture(ctx);
        return;
    }
    uint32_t width, height, stride, frame_count;
    uint64_t sequence, frame_size;
    memcpy(&width, header + 12, 4);
    memcpy(&height, header + 16, 4);
    memcpy(&stride, header + 20, 4);
    memcpy(&frame_count, header + 24, 4);
    memcpy(&sequence, header + 28, 8);
    memcpy(&frame_size, header + 36, 8);
    if (!width || !height || !stride || !frame_count || frame_size > (uint64_t)1920 * 1080 * 4 ||
        sequence == ctx->sequence) {
        fclose(file);
        clear_stale_texture(ctx);
        return;
    }
    if (ctx->pixel_capacity < frame_size) {
        ctx->pixels = brealloc(ctx->pixels, (size_t)frame_size);
        ctx->pixel_capacity = (size_t)frame_size;
    }
    uint64_t index = sequence % frame_count;
    _fseeki64(file, HEADER_SIZE + index * frame_size, SEEK_SET);
    bool complete = fread(ctx->pixels, 1, (size_t)frame_size, file) == frame_size;
    fclose(file);
    if (!complete)
        return;

    obs_enter_graphics();
    if (!ctx->texture || ctx->width != width || ctx->height != height) {
        if (ctx->texture)
            gs_texture_destroy(ctx->texture);
        ctx->texture = gs_texture_create(width, height, GS_RGBA, 1, NULL, GS_DYNAMIC);
    }
    if (ctx->texture)
        gs_texture_set_image(ctx->texture, ctx->pixels, stride, false);
    obs_leave_graphics();
    ctx->width = width;
    ctx->height = height;
    ctx->stride = stride;
    ctx->sequence = sequence;
    ctx->last_frame_ns = os_gettime_ns();
}

static uint32_t video_width(void *data) { return ((struct bbt_video *)data)->width; }
static uint32_t video_height(void *data) { return ((struct bbt_video *)data)->height; }

static void video_render(void *data, gs_effect_t *effect)
{
    UNUSED_PARAMETER(effect);
    struct bbt_video *ctx = data;
    if (!ctx->texture)
        return;
    gs_effect_t *draw = obs_get_base_effect(OBS_EFFECT_DEFAULT);
    while (gs_effect_loop(draw, "Draw"))
        gs_draw_sprite(ctx->texture, 0, ctx->width, ctx->height);
}

struct bbt_audio { obs_source_t *source; };

static const char *audio_name(void *unused)
{
    UNUSED_PARAMETER(unused);
    return obs_module_text("SharedAudio");
}

static void *audio_create(obs_data_t *settings, obs_source_t *source)
{
    UNUSED_PARAMETER(settings);
    struct bbt_audio *ctx = bzalloc(sizeof(*ctx));
    ctx->source = source;
    blog(LOG_INFO, "[Beatblock Together] Shared Audio follows the featured in-game renderer; song-only fallback is reported in Online diagnostics.");
    return ctx;
}

static void audio_destroy(void *data) { bfree(data); }

static struct obs_source_info video_info = {
    .id = "beatblock_together_player_stream",
    .type = OBS_SOURCE_TYPE_INPUT,
    .output_flags = OBS_SOURCE_VIDEO | OBS_SOURCE_CUSTOM_DRAW,
    .get_name = video_name,
    .create = video_create,
    .destroy = video_destroy,
    .get_defaults = video_defaults,
    .get_properties = video_properties,
    .update = video_update,
    .video_tick = video_tick,
    .video_render = video_render,
    .get_width = video_width,
    .get_height = video_height,
    .icon_type = OBS_ICON_TYPE_GAME_CAPTURE,
};

static struct obs_source_info audio_info = {
    .id = "beatblock_together_shared_audio",
    .type = OBS_SOURCE_TYPE_INPUT,
    .output_flags = OBS_SOURCE_AUDIO,
    .get_name = audio_name,
    .create = audio_create,
    .destroy = audio_destroy,
    .icon_type = OBS_ICON_TYPE_AUDIO_INPUT,
};

bool obs_module_load(void)
{
    obs_register_source(&video_info);
    obs_register_source(&audio_info);
    blog(LOG_INFO, "[Beatblock Together] OBS sources registered");
    return true;
}
