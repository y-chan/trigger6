use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;
use ffmpeg_next as ffmpeg;
use image::RgbImage;
use t6proto::usb::T6Device;
use t6proto::{
    DEFAULT_MAX_BULK_PACKET_SIZE, FrameScheduler, JpegFramePacket, VIDEO_COLOR_NV12,
    VIDEO_FLAG_RESET_JPEG, VramLayout,
};
use turbojpeg::Subsamp;

#[derive(Clone, Debug)]
struct Options {
    video_path: PathBuf,
    display_index: u8,
    width: u16,
    height: u16,
    fps: u32,
    frames: Option<u32>,
    quality: i32,
    ready: bool,
    power_on: bool,
    dry_run: bool,
    ram_size_mb: Option<u8>,
    max_packet_size: u32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    ffmpeg::init()?;

    let (device, ram_size_mb) = if options.dry_run {
        (None, options.ram_size_mb.unwrap_or(58))
    } else {
        let device = T6Device::open_first()?;
        let ram_size_mb = device.read_video_ram_size_mb()?;
        (Some(device), ram_size_mb)
    };

    if options.ready {
        device
            .as_ref()
            .ok_or("--ready cannot be used with --dry-run")?
            .send_software_ready(u16::from(options.display_index))?;
        println!("Sent software ready.");
    }
    if options.power_on {
        device
            .as_ref()
            .ok_or("--power-on cannot be used with --dry-run")?
            .set_monitor_power(u16::from(options.display_index), true)?;
        println!("Sent monitor power on.");
    }

    let layout = VramLayout::two_port_1080p_secondary(ram_size_mb);
    let mut scheduler = FrameScheduler::new(layout);
    let mut ictx = ffmpeg::format::input(&options.video_path)?;
    let input = ictx
        .streams()
        .best(Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?;
    let stream_index = input.index();
    let decoder_context = ffmpeg::codec::context::Context::from_parameters(input.parameters())?;
    let mut decoder = decoder_context.decoder().video()?;
    let mut scaler = Scaler::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        Pixel::RGB24,
        u32::from(options.width),
        u32::from(options.height),
        Flags::BILINEAR,
    )?;

    println!(
        "Playing video display={} out={}x{} fps={} ram={}MB dry_run={}",
        options.display_index,
        options.width,
        options.height,
        options.fps,
        ram_size_mb,
        options.dry_run
    );

    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(options.fps.max(1)));
    let started = Instant::now();
    let mut next_send_at = started;
    let mut sent_frames = 0u32;

    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }

        decoder.send_packet(&packet)?;
        receive_and_send_frames(
            &mut decoder,
            &mut scaler,
            &options,
            device.as_ref(),
            &mut scheduler,
            &mut sent_frames,
            &mut next_send_at,
            frame_interval,
        )?;

        if reached_frame_limit(sent_frames, options.frames) {
            break;
        }
    }

    if !reached_frame_limit(sent_frames, options.frames) {
        decoder.send_eof()?;
        receive_and_send_frames(
            &mut decoder,
            &mut scaler,
            &options,
            device.as_ref(),
            &mut scheduler,
            &mut sent_frames,
            &mut next_send_at,
            frame_interval,
        )?;
    }

    println!(
        "Sent {} frames in {:.3}s",
        sent_frames,
        started.elapsed().as_secs_f64()
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn receive_and_send_frames(
    decoder: &mut ffmpeg::decoder::Video,
    scaler: &mut Scaler,
    options: &Options,
    device: Option<&T6Device>,
    scheduler: &mut FrameScheduler,
    sent_frames: &mut u32,
    next_send_at: &mut Instant,
    frame_interval: Duration,
) -> Result<(), Box<dyn Error>> {
    let mut decoded = Video::empty();

    while decoder.receive_frame(&mut decoded).is_ok() {
        if reached_frame_limit(*sent_frames, options.frames) {
            return Ok(());
        }

        let mut rgb_frame = Video::empty();
        scaler.run(&decoded, &mut rgb_frame)?;
        let rgb = copy_rgb24_frame(
            &rgb_frame,
            u32::from(options.width),
            u32::from(options.height),
        )?;
        let jpeg = turbojpeg::compress_image(&rgb, options.quality, Subsamp::Sub2x2)?;
        let addresses = scheduler.next_jpeg_frame(jpeg.len());
        let flags = if addresses.reset_jpeg {
            VIDEO_FLAG_RESET_JPEG
        } else {
            0
        };
        let packet = JpegFramePacket::new_with_target_format(
            options.display_index,
            &jpeg,
            options.width,
            options.height,
            addresses.cmd_addr,
            addresses.fb_addr,
            VIDEO_COLOR_NV12,
            flags,
        );

        if let Some(delay) = next_send_at.checked_duration_since(Instant::now()) {
            thread::sleep(delay);
        }

        if let Some(device) = device {
            device.send_jpeg_frame(&packet, options.max_packet_size)?;
        }

        *sent_frames += 1;
        *next_send_at += frame_interval;

        if *sent_frames == 1 || *sent_frames % 30 == 0 {
            println!(
                "frame={} jpeg_bytes={} cmd=0x{:08x} fb=0x{:08x} reset={} chunks={}",
                sent_frames,
                jpeg.len(),
                addresses.cmd_addr,
                addresses.fb_addr,
                addresses.reset_jpeg,
                packet.bulk_chunks(options.max_packet_size).len()
            );
        }
    }

    Ok(())
}

fn copy_rgb24_frame(frame: &Video, width: u32, height: u32) -> Result<RgbImage, Box<dyn Error>> {
    let row_bytes = width as usize * 3;
    let stride = frame.stride(0);
    let data = frame.data(0);
    let mut rgb = Vec::with_capacity(row_bytes * height as usize);

    for y in 0..height as usize {
        let start = y * stride;
        let end = start + row_bytes;
        if end > data.len() {
            return Err("decoded RGB frame is shorter than expected".into());
        }
        rgb.extend_from_slice(&data[start..end]);
    }

    RgbImage::from_raw(width, height, rgb).ok_or_else(|| "invalid RGB frame size".into())
}

fn reached_frame_limit(sent_frames: u32, frames: Option<u32>) -> bool {
    frames
        .map(|frame_limit| sent_frames >= frame_limit)
        .unwrap_or(false)
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut video_path = None;
    let mut display_index = 1;
    let mut width = 1920;
    let mut height = 1080;
    let mut fps = 60;
    let mut frames = None;
    let mut quality = 95;
    let mut ready = false;
    let mut power_on = false;
    let mut dry_run = false;
    let mut ram_size_mb = None;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--video" => video_path = Some(PathBuf::from(next_value(&mut args, "--video")?)),
            "--display" => display_index = next_value(&mut args, "--display")?.parse()?,
            "--width" => width = next_value(&mut args, "--width")?.parse()?,
            "--height" => height = next_value(&mut args, "--height")?.parse()?,
            "--fps" => fps = next_value(&mut args, "--fps")?.parse()?,
            "--frames" => frames = Some(next_value(&mut args, "--frames")?.parse()?),
            "--quality" => quality = next_value(&mut args, "--quality")?.parse()?,
            "--ready" => ready = true,
            "--power-on" => power_on = true,
            "--dry-run" => dry_run = true,
            "--ram-size-mb" => ram_size_mb = Some(next_value(&mut args, "--ram-size-mb")?.parse()?),
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }

    if !(1..=100).contains(&quality) {
        return Err("--quality must be 1..100".into());
    }

    Ok(Options {
        video_path: video_path.ok_or("--video is required")?,
        display_index,
        width,
        height,
        fps,
        frames,
        quality,
        ready,
        power_on,
        dry_run,
        ram_size_mb,
        max_packet_size,
    })
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("{name} requires a value").into())
}

fn parse_u32(value: &str) -> Result<u32, Box<dyn Error>> {
    if let Some(hex) = value.strip_prefix("0x") {
        Ok(u32::from_str_radix(hex, 16)?)
    } else {
        Ok(value.parse()?)
    }
}

fn print_help() {
    println!(
        "Usage: t6-play-video --video input.mp4 [options]\n\
\n\
Options:\n\
    --display 0|1       Display index, default 1\n\
    --width N           Output width, default 1920\n\
    --height N          Output height, default 1080\n\
    --fps N             Send rate, default 60\n\
    --frames N          Stop after N output frames\n\
    --quality N         TurboJPEG quality 1..100, default 95\n\
    --ready             Send software-ready before playback\n\
    --power-on          Send monitor power-on before playback\n\
    --dry-run           Decode/encode/packetize but do not open USB or send\n\
    --ram-size-mb N     RAM size for dry-run address planning, default 58\n\
    --max-packet N      Bulk fragment size, default 0x19000"
    );
}
