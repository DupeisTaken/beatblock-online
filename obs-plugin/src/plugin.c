#include <obs-module.h>
#include <graphics/graphics.h>
#include <util/platform.h>
#include <windows.h>
#include <stdint.h>
#include <stdio.h>

OBS_DECLARE_MODULE()
OBS_MODULE_USE_DEFAULT_LOCALE("beatblock-online-obs", "en-US")

#define HEADER_SIZE 64
#define FRAME_MAGIC "BBTFRAME"
#define FRAME_VERSION 2
#define MAPPING_RETRY_NS 500000000ULL
#define STALE_FRAME_NS 1500000000ULL

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
// unchanged. The sequence field is 8-byte aligned and the plugin is x64-only,
// so aligned loads are atomic; barriers keep the two snapshots around the
// pixel copy from being reordered.
static uint64_t read_committed_sequence(const uint8_t *header)
{
    uint64_t sequence;
    MemoryBarrier();
    memcpy(&sequence, header + 32, sizeof(sequence));
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
    char requested_slot = slot && *slot ? slot[0] : 'A';
    if (requested_slot >= 'a' && requested_slot <= 'd')
        requested_slot -= ('a' - 'A');
    if (requested_slot < 'A' || requested_slot > 'D')
        requested_slot = 'A';
    if (ctx->slot == requested_slot && ctx->path[0])
        return;
    ctx->slot = requested_slot;
    build_path(ctx);
    close_frame_mapping(ctx);
    clear_video_frame(ctx);
    ctx->sequence = 0;
    ctx->last_frame_ns = 0;
    ctx->next_mapping_attempt_ns = 0;
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
    uint32_t version;
    memcpy(&version, header + 8, 4);
    if (memcmp(header, FRAME_MAGIC, 8) != 0 || version != FRAME_VERSION) {
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
    sequence = read_committed_sequence(header);
    memcpy(&frame_size, header + 40, 8);
    if (!width || width > 1920 || !height || height > 1080 || stride != width * 4 ||
        !frame_count || frame_count > 3 || !sequence || frame_size != (uint64_t)stride * height ||
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
    uint64_t offset = HEADER_SIZE + index * frame_size;
    if (offset > ctx->mapped_size || frame_size > ctx->mapped_size - offset) {
        retry_frame_mapping_later(ctx);
        return;
    }
    memcpy(ctx->pixels, ctx->mapped + offset, (size_t)frame_size);
    MemoryBarrier();
    uint64_t confirmed = read_committed_sequence(header);
    if (confirmed != sequence)
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
    if (!draw)
        return;
    // Custom-draw sources own the base effect setup. gs_draw_sprite only emits
    // geometry; without binding the texture to `image`, OBS renders a correctly
    // sized black rectangle even though the shared-memory pixels are valid.
    gs_eparam_t *image = gs_effect_get_param_by_name(draw, "image");
    if (!image)
        return;
    gs_effect_set_texture_srgb(image, ctx->texture);
    while (gs_effect_loop(draw, "Draw"))
        gs_draw_sprite(ctx->texture, 0, ctx->width, ctx->height);
}

static struct obs_source_info video_info = {
    .id = "beatblock_online_player_stream",
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

bool obs_module_load(void)
{
    obs_register_source(&video_info);
    blog(LOG_INFO, "[Beatblock Online] OBS player stream source registered");
    return true;
}
