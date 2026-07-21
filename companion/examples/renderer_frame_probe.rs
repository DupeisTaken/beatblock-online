//! Physical renderer probe for the isolated `.test` Beatblock build.
//!
//! Run explicitly with `BBT_PROBE_GAME` pointing at Beatblock.exe. The probe
//! launches one hidden renderer, drives a playing sample, and validates the
//! committed RGBA frame instead of treating a rising sequence as useful video.

use anyhow::{bail, Context, Result};
use beatblock_online_companion::{
    model::{RenderSample, RendererMode, RendererRequest},
    renderer::{prepare_renderer_profile, RendererManager},
    room::unix_ms,
};
use std::{
    collections::HashSet,
    env,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const HEADER_SIZE: usize = 64;
type CommittedFrame = (u64, u32, u32, Vec<u8>);

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .context("renderer frame header is truncated")?;
    Ok(u32::from_le_bytes(value.try_into()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let value = bytes
        .get(offset..offset + 8)
        .context("renderer frame header is truncated")?;
    Ok(u64::from_le_bytes(value.try_into()?))
}

fn read_committed_frame(path: &Path) -> Result<Option<CommittedFrame>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file_len = file.metadata()?.len();
    if file_len < HEADER_SIZE as u64 {
        return Ok(None);
    }
    let mut header = [0u8; HEADER_SIZE];
    file.read_exact(&mut header)?;
    if &header[..8] != b"BBTFRAME" || read_u32(&header, 8)? != 2 {
        bail!("renderer frame has an unsupported header");
    }
    let width = read_u32(&header, 12)?;
    let height = read_u32(&header, 16)?;
    let stride = read_u32(&header, 20)?;
    let frame_count = read_u32(&header, 24)?;
    let sequence = read_u64(&header, 32)?;
    let frame_size = read_u64(&header, 40)?;
    if sequence < 3 {
        return Ok(None);
    }
    if width == 0
        || width > 1920
        || height == 0
        || height > 1080
        || stride != width * 4
        || !(1..=3).contains(&frame_count)
        || frame_size != u64::from(stride) * u64::from(height)
    {
        bail!("renderer frame metadata is invalid");
    }
    let offset = (HEADER_SIZE as u64)
        .checked_add(
            (sequence % u64::from(frame_count))
                .checked_mul(frame_size)
                .context("renderer frame offset overflowed")?,
        )
        .context("renderer frame offset overflowed")?;
    let end = offset
        .checked_add(frame_size)
        .context("renderer frame extent overflowed")?;
    if end > file_len {
        bail!("renderer frame pixels are truncated");
    }

    let mut pixels = vec![0u8; usize::try_from(frame_size)?];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut pixels)?;
    // Mirror the OBS reader's commit check: discard a slot that changed while
    // its pixels were copied rather than diagnosing a torn producer frame.
    file.seek(SeekFrom::Start(32))?;
    let mut confirmation = [0u8; 8];
    file.read_exact(&mut confirmation)?;
    if u64::from_le_bytes(confirmation) != sequence {
        return Ok(None);
    }
    Ok(Some((sequence, width, height, pixels)))
}

fn push_playing_sample(
    manager: &RendererManager,
    started: &Instant,
    start_beat: f32,
    beats_per_second: f32,
) -> f32 {
    let now_us = unix_ms() * 1_000;
    // Tutorial begins at beat -8 and reaches its Play Song event at beat 0
    // using 150 BPM. Driving that real pre-roll prevents the probe itself from
    // collapsing sixteen beats of chart/VFX events into its first frame.
    let beat = start_beat + started.elapsed().as_secs_f32() * beats_per_second;
    manager.push_sample(
        "physical-probe",
        RenderSample {
            session_id: 1,
            sequence: started.elapsed().as_millis() as u32 + 1,
            run_time_us: now_us,
            beat,
            paddle_angle: 45.0,
            tap_mask: 0,
            flags: 1,
        },
    );
    manager.write_aligned_inputs(now_us);
    beat
}

fn write_bmp(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let pixel_bytes = usize::try_from(u64::from(width) * u64::from(height) * 4)?;
    if rgba.len() != pixel_bytes {
        bail!("renderer snapshot length does not match its dimensions");
    }
    let file_size = 54usize
        .checked_add(pixel_bytes)
        .context("renderer BMP size overflowed")?;
    let mut header = [0u8; 54];
    header[0..2].copy_from_slice(b"BM");
    header[2..6].copy_from_slice(&u32::try_from(file_size)?.to_le_bytes());
    header[10..14].copy_from_slice(&54u32.to_le_bytes());
    header[14..18].copy_from_slice(&40u32.to_le_bytes());
    header[18..22].copy_from_slice(&width.to_le_bytes());
    header[22..26].copy_from_slice(&height.to_le_bytes());
    header[26..28].copy_from_slice(&1u16.to_le_bytes());
    header[28..30].copy_from_slice(&32u16.to_le_bytes());
    header[34..38].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());

    let mut file = File::create(path)?;
    file.write_all(&header)?;
    // Windows BMP rows are bottom-up and use BGRA byte order.
    for row in (0..height as usize).rev() {
        for pixel in rgba[row * width as usize * 4..(row + 1) * width as usize * 4].chunks_exact(4)
        {
            file.write_all(&[pixel[2], pixel[1], pixel[0], pixel[3]])?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let game = env::var_os("BBT_PROBE_GAME")
        .map(PathBuf::from)
        .context("set BBT_PROBE_GAME to the isolated Beatblock.exe")?;
    let mode = match env::var("BBT_PROBE_MODE")
        .unwrap_or_else(|_| "clean".into())
        .to_ascii_lowercase()
        .as_str()
    {
        "clean" => RendererMode::Clean,
        "full" => RendererMode::Full,
        other => bail!("unsupported BBT_PROBE_MODE {other:?}"),
    };
    let root = env::var_os("BBT_PROBE_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("beatblock-online-renderer-probe"));
    let capture_beat = env::var("BBT_PROBE_CAPTURE_BEAT")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && (-64.0..=4_096.0).contains(value))
        .unwrap_or(1.0);
    let start_beat = env::var("BBT_PROBE_START_BEAT")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(-8.0);
    let beats_per_second = env::var("BBT_PROBE_BEATS_PER_SECOND")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0 && *value <= 32.0)
        .unwrap_or(2.5);
    std::fs::create_dir_all(&root)?;

    let manager = RendererManager::new(root.clone())?;
    manager.configure(
        "A",
        RendererRequest {
            participant_id: Some("physical-probe".into()),
            participant_name: Some("Physical probe".into()),
            mode: Some(mode),
            width: Some(1280),
            height: Some(720),
            fps: Some(30),
            delay_ms: Some(250),
            featured: Some(true),
        },
    )?;
    let profile = prepare_renderer_profile(&root)?;
    manager.launch_slot(
        "A",
        &game,
        &profile,
        "levels/Finished levels/tutorial/",
        "easy",
    )?;
    // Tutorial's first scoring interaction is at beat 0. The production game
    // publishes this once after parsing Event.hitCount; the probe supplies the
    // same anchor explicitly so it exercises the first-note release barrier.
    manager.push_render_anchor("physical-probe", 0.0, 0.0);

    let started = Instant::now();
    let frame = loop {
        let beat = push_playing_sample(&manager, &started, start_beat, beats_per_second);
        if beat >= capture_beat {
            if let Some(frame) = read_committed_frame(&manager.frame_path("A"))? {
                break frame;
            }
        }
        if let Ok(error) = std::fs::read_to_string(manager.error_path("A")) {
            bail!("renderer reported: {error}");
        }
        if started.elapsed() > Duration::from_secs(30) {
            bail!("renderer did not publish three frames within 30 seconds");
        }
        thread::sleep(Duration::from_millis(50));
    };

    let (sequence, width, height, pixels) = frame;
    let mut non_black = 0usize;
    let mut opaque = 0usize;
    let mut colors = HashSet::new();
    let mut luma_sum = 0f64;
    let mut luma_sq_sum = 0f64;
    let mut tile_min = [255f64; 32];
    let mut tile_max = [0f64; 32];
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        let luma = 0.2126 * pixel[0] as f64 + 0.7152 * pixel[1] as f64 + 0.0722 * pixel[2] as f64;
        luma_sum += luma;
        luma_sq_sum += luma * luma;
        non_black += usize::from(pixel[0] > 8 || pixel[1] > 8 || pixel[2] > 8);
        opaque += usize::from(pixel[3] >= 250);
        colors.insert(
            ((pixel[0] as u16 >> 4) << 8) | ((pixel[1] as u16 >> 4) << 4) | (pixel[2] as u16 >> 4),
        );
        let x = index % width as usize;
        let y = index / width as usize;
        let tile = (y * 4 / height as usize) * 8 + x * 8 / width as usize;
        tile_min[tile] = tile_min[tile].min(luma);
        tile_max[tile] = tile_max[tile].max(luma);
    }
    let count = (width as usize * height as usize) as f64;
    let mean = luma_sum / count;
    let stddev = (luma_sq_sum / count - mean * mean).max(0.0).sqrt();
    let non_black_ratio = non_black as f64 / count;
    let opaque_ratio = opaque as f64 / count;
    let active_tiles = tile_min
        .iter()
        .zip(tile_max)
        .filter(|(minimum, maximum)| *maximum - **minimum >= 20.0)
        .count();
    let image_path = root.join(format!("renderer-probe-{mode:?}.bmp").to_ascii_lowercase());
    write_bmp(&image_path, width, height, &pixels)?;

    println!(
        "sequence={sequence} mode={mode:?} size={width}x{height} non_black={non_black_ratio:.4} \
         opaque={opaque_ratio:.4} luma_mean={mean:.2} luma_stddev={stddev:.2} coarse_colors={} \
         active_tiles={active_tiles} image={}",
        colors.len(),
        image_path.display()
    );
    if opaque_ratio < 0.99 {
        bail!("renderer output is not opaque enough for OBS");
    }
    if non_black_ratio < 0.03 || stddev < 8.0 || colors.len() < 2 || active_tiles < 8 {
        bail!("renderer output lacks meaningful visual content");
    }
    let hold_seconds = env::var("BBT_PROBE_HOLD_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let hold_started = Instant::now();
    while hold_started.elapsed() < Duration::from_secs(hold_seconds) {
        push_playing_sample(&manager, &started, start_beat, beats_per_second);
        thread::sleep(Duration::from_millis(16));
    }
    manager.stop_all();
    Ok(())
}
