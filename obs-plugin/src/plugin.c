#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define AUDIO_WINDOW_FORMAT "Beatblock Online Renderer %s:SDL_app:Beatblock.exe"
#define AUTOPLAY_AUDIO_WINDOW "Beatblock Online Autoplay:SDL_app:Beatblock.exe"
#define DEFAULT_AUDIO_SYNC_MS 0
#define HEADER_SIZE 80
#define FRAME_MAGIC "BBTFRAME"
#define FRAME_VERSION 4
#define FRAME_PIXEL_FORMAT_OFFSET 28
#define FRAME_PIXEL_FORMAT_RGBA8_DISPLAY_SRGB 1
#define FRAME_SLOT_SEQUENCE_OFFSET 56
#define MAX_FRAME_COUNT 3

static const char *normalize_audio_target(const char *target)
{
    if (!target || !*target)
        return "A";
    if (strcmp(target, "AUTOPLAY") == 0 || strcmp(target, "autoplay") == 0)
        return "AUTOPLAY";
    if (target[1] == '\0' && target[0] >= 'a' && target[0] <= 'd') {
        static const char *lower_targets[] = {"A", "B", "C", "D"};
        return lower_targets[target[0] - 'a'];
    }
    if (target[1] == '\0' && target[0] >= 'A' && target[0] <= 'D') {
        static const char *upper_targets[] = {"A", "B", "C", "D"};
        return upper_targets[target[0] - 'A'];
    }
    return "A";
}

static char normalize_stream_slot(char slot)
{
    char target[2] = {slot, '\0'};
    return normalize_audio_target(target)[0];
}

static void build_audio_window(const char *target, char *window, size_t size)
{
    const char *normalized = normalize_audio_target(target);
    if (strcmp(normalized, "AUTOPLAY") == 0)
        snprintf(window, size, "%s", AUTOPLAY_AUDIO_WINDOW);
    else
        snprintf(window, size, AUDIO_WINDOW_FORMAT, normalized);
}

static uint32_t read_header_u32(const uint8_t *header, size_t offset)
{
    uint32_t value;
    memcpy(&value, header + offset, sizeof(value));
    return value;
}

static uint64_t read_header_u64(const uint8_t *header, size_t offset)
{
    uint64_t value;
    memcpy(&value, header + offset, sizeof(value));
    return value;
}

static bool frame_header_has_supported_pixels(const uint8_t *header)
{
    return memcmp(header, FRAME_MAGIC, 8) == 0 &&
        read_header_u32(header, 8) == FRAME_VERSION &&
        read_header_u32(header, FRAME_PIXEL_FORMAT_OFFSET) ==
        FRAME_PIXEL_FORMAT_RGBA8_DISPLAY_SRGB;
}

static size_t frame_slot_sequence_offset(uint64_t sequence, uint32_t frame_count)
{
    return FRAME_SLOT_SEQUENCE_OFFSET +
        (size_t)(sequence % frame_count) * sizeof(uint64_t);
}

// Header v4 gives each modulo slot its own generation. The producer clears the
// marker before reuse and commits it only after all pixels are present. Checking
// the marker on both sides of a copy closes the N/N+3 race that a global
// sequence snapshot alone cannot detect.
static bool frame_slot_has_committed_sequence(
    const uint8_t *header, uint32_t frame_count, uint64_t sequence)
{
    if (!sequence || !frame_count || frame_count > MAX_FRAME_COUNT)
        return false;
    return read_header_u64(header,
        frame_slot_sequence_offset(sequence, frame_count)) == sequence;
}

static bool frame_header_has_committed_frame(
    const uint8_t *header, uint32_t frame_count, uint64_t sequence)
{
    return read_header_u64(header, 32) == sequence &&
        frame_slot_has_committed_sequence(header, frame_count, sequence);
}

#ifndef BBT_AUDIO_TARGET_TEST

#include <obs-module.h>
#include <graphics/graphics.h>
#include <util/platform.h>
#include <windows.h>

OBS_DECLARE_MODULE()
OBS_MODULE_USE_DEFAULT_LOCALE("beatblock-online-obs", "en-US")

#define MAPPING_RETRY_NS 500000000ULL
#define STALE_FRAME_NS 1500000000ULL
#define PROCESS_AUDIO_SOURCE_ID "wasapi_process_output_capture"
#define OBS_WINDOW_PRIORITY_TITLE 0

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
    uint64_t next_mapping_attempt_ns;
    uint64_t mapping_opened_ns;
    HANDLE frame_file;
    HANDLE frame_mapping;
    const uint8_t *mapped;
    size_t mapped_size;
    wchar_t path[MAX_PATH * 2];
};

enum bbt_audio_status {
    BBT_AUDIO_CONNECTING,
    BBT_AUDIO_CONNECTED,
    BBT_AUDIO_UNAVAILABLE,
};

struct bbt_audio {
    obs_source_t *source;
    obs_source_t *capture;
    char target[16];
    enum bbt_audio_status status;
};

static bool reroute_audio_source(obs_source_t *audio_source, obs_source_t *target)
{
    calldata_t params = {0};
    calldata_set_ptr(&params, "target", target);
    bool routed = proc_handler_call(obs_source_get_proc_handler(audio_source),
        "reroute_audio", &params);
    calldata_free(&params);
    return routed;
}

static void destroy_audio_capture(struct bbt_audio *ctx)
{
    obs_source_set_audio_active(ctx->source, false);
    if (!ctx->capture)
        return;
    obs_source_set_sync_offset(ctx->capture, 0);
    reroute_audio_source(ctx->capture, NULL);
    obs_source_remove_active_child(ctx->source, ctx->capture);
    obs_source_release(ctx->capture);
    ctx->capture = NULL;
}

static void update_audio_capture(struct bbt_audio *ctx, obs_data_t *settings)
{
    obs_source_set_sync_offset(ctx->source, 0);
    const char *requested = normalize_audio_target(
        obs_data_get_string(settings, "target"));
    snprintf(ctx->target, sizeof(ctx->target), "%s", requested);
    char window[128];
    build_audio_window(ctx->target, window, sizeof(window));
    obs_data_t *audio_settings = obs_data_create();
    obs_data_set_string(audio_settings, "window", window);
    obs_data_set_int(audio_settings, "priority", OBS_WINDOW_PRIORITY_TITLE);

    if (!ctx->capture) {
        char name[256];
        snprintf(name, sizeof(name), "%s (private process capture)",
            obs_source_get_name(ctx->source));
        ctx->capture = obs_source_create_private(PROCESS_AUDIO_SOURCE_ID,
            name, audio_settings);
        if (!ctx->capture) {
            blog(LOG_WARNING,
                "[Beatblock Online] OBS Application Audio Capture is unavailable; "
                "this Beatblock Online Audio source is silent");
            obs_data_release(audio_settings);
            ctx->status = BBT_AUDIO_UNAVAILABLE;
            obs_source_set_audio_active(ctx->source, false);
            return;
        }
        if (!obs_source_add_active_child(ctx->source, ctx->capture) ||
            !reroute_audio_source(ctx->capture, ctx->source)) {
            blog(LOG_WARNING,
                "[Beatblock Online] OBS Application Audio Capture could not be "
                "attached to this Beatblock Online Audio source");
            destroy_audio_capture(ctx);
            obs_data_release(audio_settings);
            ctx->status = BBT_AUDIO_UNAVAILABLE;
            return;
        }
    } else {
        obs_source_update(ctx->capture, audio_settings);
    }
    obs_data_release(audio_settings);

    int64_t sync_ms = obs_data_get_int(settings, "audio_sync_ms");
    if (sync_ms < 0)
        sync_ms = 0;
    if (sync_ms > 2000)
        sync_ms = 2000;
    obs_source_set_sync_offset(ctx->capture, sync_ms * 1000000LL);
    obs_source_set_audio_active(ctx->source, true);
    ctx->status = BBT_AUDIO_CONNECTED;
}

static void close_frame_mapping(struct bbt_video *ctx)
{
    if (ctx->mapped)
        UnmapViewOfFile(ctx->mapped);
    if (ctx->frame_mapping)
        CloseHandle(ctx->frame_mapping);
    if (ctx->frame_file && ctx->frame_file != INVALID_HANDLE_VALUE)
        CloseHandle(ctx->frame_file);
    ctx->mapped = NULL;
    ctx->frame_mapping = NULL;
    ctx->frame_file = NULL;
    ctx->mapped_size = 0;
    ctx->mapping_opened_ns = 0;
}

static void retry_frame_mapping_later(struct bbt_video *ctx)
{
    close_frame_mapping(ctx);
    ctx->next_mapping_attempt_ns = os_gettime_ns() + MAPPING_RETRY_NS;
}

static bool ensure_frame_mapping(struct bbt_video *ctx)
{
    if (ctx->mapped)
        return true;
    uint64_t now = os_gettime_ns();
    if (now < ctx->next_mapping_attempt_ns)
        return false;
    HANDLE file = CreateFileW(ctx->path, GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL, OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) {
        ctx->next_mapping_attempt_ns = now + MAPPING_RETRY_NS;
        return false;
    }
    LARGE_INTEGER size;
    if (!GetFileSizeEx(file, &size) || size.QuadPart < HEADER_SIZE ||
        (uint64_t)size.QuadPart > SIZE_MAX) {
        CloseHandle(file);
        ctx->next_mapping_attempt_ns = now + MAPPING_RETRY_NS;
        return false;
    }
    HANDLE mapping = CreateFileMappingW(file, NULL, PAGE_READONLY, 0, 0, NULL);
    if (!mapping) {
        CloseHandle(file);
        ctx->next_mapping_attempt_ns = now + MAPPING_RETRY_NS;
        return false;
    }
    const uint8_t *mapped = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
    if (!mapped) {
        CloseHandle(mapping);
        CloseHandle(file);
        ctx->next_mapping_attempt_ns = now + MAPPING_RETRY_NS;
        return false;
    }
    ctx->frame_file = file;
    ctx->frame_mapping = mapping;
    ctx->mapped = mapped;
    ctx->mapped_size = (size_t)size.QuadPart;
    ctx->next_mapping_attempt_ns = 0;
    ctx->mapping_opened_ns = now;
    return true;
}

// The frame view is deliberately FILE_MAP_READ. A locked interlocked
// read-modify-write faults on that mapping even when the exchange value is
// unchanged. Every v4 commit field is 8-byte aligned and the plugin is x64-only,
// so aligned loads are atomic; barriers keep snapshots around the pixel copy
// from being reordered.
static uint64_t read_committed_sequence(const uint8_t *header, size_t offset)
{
    uint64_t sequence;
    MemoryBarrier();
    memcpy(&sequence, header + offset, sizeof(sequence));
    MemoryBarrier();
    return sequence;
}

static void clear_video_frame(struct bbt_video *ctx)
{
    obs_enter_graphics();
    if (ctx->texture)
        gs_texture_destroy(ctx->texture);
    ctx->texture = NULL;
    obs_leave_graphics();
    ctx->width = 0;
    ctx->height = 0;
    bfree(ctx->pixels);
    ctx->pixels = NULL;
    ctx->pixel_capacity = 0;
}

static void clear_stale_resources(struct bbt_video *ctx)
{
    uint64_t now = os_gettime_ns();
    if (!ctx->last_frame_ns) {
        if (ctx->mapped && ctx->mapping_opened_ns &&
            now - ctx->mapping_opened_ns >= STALE_FRAME_NS)
            retry_frame_mapping_later(ctx);
        return;
    }
    if (now - ctx->last_frame_ns < STALE_FRAME_NS)
        return;
    clear_video_frame(ctx);
    // A mapped view continues pointing at the old file after an atomic file
    // replacement. Reopen after a quiet renderer so a restarted slot can
    // publish through a newly-created backing file.
    retry_frame_mapping_later(ctx);
    ctx->last_frame_ns = 0;
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
        L"%s\\BeatblockOnline\\BeatblockOnline\\data\\render-streams\\stream-%c.bbtframe",
        root, ctx->slot);
}

static void video_update(void *data, obs_data_t *settings)
{
    struct bbt_video *ctx = data;
    const char *slot = obs_data_get_string(settings, "slot");
    char requested_slot = normalize_stream_slot(slot && *slot ? slot[0] : 'A');
    if (ctx->slot != requested_slot || !ctx->path[0]) {
        ctx->slot = requested_slot;
        build_path(ctx);
        close_frame_mapping(ctx);
        clear_video_frame(ctx);
        ctx->sequence = 0;
        ctx->last_frame_ns = 0;
        ctx->next_mapping_attempt_ns = 0;
    }
}

static void *video_create(obs_data_t *settings, obs_source_t *source)
{
    struct bbt_video *ctx = bzalloc(sizeof(*ctx));
    ctx->source = source;
    video_update(ctx, settings);
    return ctx;
}

static void video_destroy(void *data)
{
    struct bbt_video *ctx = data;
    clear_video_frame(ctx);
    close_frame_mapping(ctx);
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
    obs_properties_add_text(props, "audio_migration", obs_module_text("AudioMigration"),
        OBS_TEXT_INFO);
    return props;
}

static void video_tick(void *data, float seconds)
{
    UNUSED_PARAMETER(seconds);
    struct bbt_video *ctx = data;
    if (!ensure_frame_mapping(ctx)) {
        clear_stale_resources(ctx);
        return;
    }
    const uint8_t *header = ctx->mapped;
    if (!frame_header_has_supported_pixels(header)) {
        retry_frame_mapping_later(ctx);
        clear_stale_resources(ctx);
        return;
    }
    uint32_t width, height, stride, frame_count;
    uint64_t sequence, frame_size;
    memcpy(&width, header + 12, 4);
    memcpy(&height, header + 16, 4);
    memcpy(&stride, header + 20, 4);
    memcpy(&frame_count, header + 24, 4);
    sequence = read_committed_sequence(header, 32);
    memcpy(&frame_size, header + 40, 8);
    if (!width || width > 1920 || !height || height > 1080 || stride != width * 4 ||
        !frame_count || frame_count > MAX_FRAME_COUNT || !sequence ||
        frame_size != (uint64_t)stride * height ||
        frame_size > (uint64_t)1920 * 1080 * 4 ||
        sequence == ctx->sequence) {
        clear_stale_resources(ctx);
        return;
    }
    if (ctx->pixel_capacity < frame_size) {
        ctx->pixels = brealloc(ctx->pixels, (size_t)frame_size);
        ctx->pixel_capacity = (size_t)frame_size;
    }
    uint64_t index = sequence % frame_count;
    size_t slot_sequence_offset = frame_slot_sequence_offset(sequence, frame_count);
    if (read_committed_sequence(header, slot_sequence_offset) != sequence) {
        clear_stale_resources(ctx);
        return;
    }
    uint64_t offset = HEADER_SIZE + index * frame_size;
    if (offset > ctx->mapped_size || frame_size > ctx->mapped_size - offset) {
        retry_frame_mapping_later(ctx);
        return;
    }
    memcpy(ctx->pixels, ctx->mapped + offset, (size_t)frame_size);
    MemoryBarrier();
    uint64_t confirmed_slot = read_committed_sequence(header, slot_sequence_offset);
    uint64_t confirmed_sequence = read_committed_sequence(header, 32);
    if (confirmed_slot != sequence || confirmed_sequence != sequence)
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
    obs_source_draw(ctx->texture, 0, 0, ctx->width, ctx->height, false);
}

static enum gs_color_space video_color_space(
    void *data, size_t count, const enum gs_color_space *preferred_spaces)
{
    UNUSED_PARAMETER(data);
    for (size_t index = 0; index < count; index++) {
        if (preferred_spaces[index] == GS_CS_SRGB)
            return GS_CS_SRGB;
    }
    return GS_CS_SRGB;
}

static struct obs_source_info video_info = {
    .id = "beatblock_online_player_stream",
    .type = OBS_SOURCE_TYPE_INPUT,
    .output_flags = OBS_SOURCE_VIDEO | OBS_SOURCE_SRGB,
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
    .video_get_color_space = video_color_space,
    .icon_type = OBS_ICON_TYPE_GAME_CAPTURE,
};

static const char *audio_name(void *unused)
{
    UNUSED_PARAMETER(unused);
    return obs_module_text("AudioSource");
}

static void audio_update(void *data, obs_data_t *settings)
{
    update_audio_capture(data, settings);
}

static void audio_tick(void *data, float seconds)
{
    UNUSED_PARAMETER(seconds);
    struct bbt_audio *ctx = data;
    if (!ctx->capture || ctx->status == BBT_AUDIO_UNAVAILABLE)
        return;
    calldata_t status = {0};
    bool queried = proc_handler_call(obs_source_get_proc_handler(ctx->capture),
        "get_hooked", &status);
    ctx->status = queried && calldata_bool(&status, "hooked")
        ? BBT_AUDIO_CONNECTED
        : BBT_AUDIO_CONNECTING;
    calldata_free(&status);
}

static void *audio_create(obs_data_t *settings, obs_source_t *source)
{
    struct bbt_audio *ctx = bzalloc(sizeof(*ctx));
    ctx->source = source;
    ctx->status = BBT_AUDIO_CONNECTING;
    audio_update(ctx, settings);
    return ctx;
}

static void audio_destroy(void *data)
{
    struct bbt_audio *ctx = data;
    destroy_audio_capture(ctx);
    bfree(ctx);
}

static void audio_defaults(obs_data_t *settings)
{
    obs_data_set_default_string(settings, "target", "A");
    obs_data_set_default_int(settings, "audio_sync_ms", DEFAULT_AUDIO_SYNC_MS);
}

static obs_properties_t *audio_properties(void *data)
{
    struct bbt_audio *ctx = data;
    obs_properties_t *props = obs_properties_create();
    obs_property_t *target = obs_properties_add_list(props, "target",
        obs_module_text("AudioTarget"), OBS_COMBO_TYPE_LIST,
        OBS_COMBO_FORMAT_STRING);
    obs_property_list_add_string(target, "Stream A", "A");
    obs_property_list_add_string(target, "Stream B", "B");
    obs_property_list_add_string(target, "Stream C", "C");
    obs_property_list_add_string(target, "Stream D", "D");
    obs_property_list_add_string(target, "Autoplay", "AUTOPLAY");
    obs_properties_add_int_slider(props, "audio_sync_ms",
        obs_module_text("AudioSync"), 0, 2000, 50);

    const char *status = "AudioConnecting";
    if (ctx && ctx->status == BBT_AUDIO_CONNECTED)
        status = "AudioConnected";
    else if (ctx && ctx->status == BBT_AUDIO_UNAVAILABLE)
        status = "AudioUnavailable";
    obs_properties_add_text(props, "connection_status",
        obs_module_text(status), OBS_TEXT_INFO);
    obs_properties_add_text(props, "audio_hint", obs_module_text("AudioHint"),
        OBS_TEXT_INFO);
    return props;
}

static struct obs_source_info audio_info = {
    .id = "beatblock_online_audio",
    .type = OBS_SOURCE_TYPE_INPUT,
    .output_flags = OBS_SOURCE_AUDIO | OBS_SOURCE_DO_NOT_DUPLICATE |
        OBS_SOURCE_DO_NOT_SELF_MONITOR,
    .get_name = audio_name,
    .create = audio_create,
    .destroy = audio_destroy,
    .get_defaults = audio_defaults,
    .get_properties = audio_properties,
    .update = audio_update,
    .video_tick = audio_tick,
    .icon_type = OBS_ICON_TYPE_PROCESS_AUDIO_OUTPUT,
};

bool obs_module_load(void)
{
    obs_register_source(&video_info);
    obs_register_source(&audio_info);
    blog(LOG_INFO,
        "[Beatblock Online] OBS player stream and audio sources registered");
    return true;
}

#endif
