use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use t6proto::usb::T6Device;
use t6proto::{
    DEFAULT_MAX_BULK_PACKET_SIZE, FrameScheduler, JpegFramePacket, VIDEO_COLOR_NV12,
    VIDEO_FLAG_RESET_JPEG, VramLayout,
};

#[derive(Clone, Debug)]
struct Options {
    jpeg_path: PathBuf,
    display_index: u8,
    width: u16,
    height: u16,
    frames: u32,
    fps: u32,
    ready: bool,
    power_on: bool,
    max_packet_size: u32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let jpeg = fs::read(&options.jpeg_path)?;
    let device = T6Device::open_first()?;
    let ram_size_mb = device.read_video_ram_size_mb()?;
    let layout = VramLayout::two_port_1080p_secondary(ram_size_mb);
    let mut scheduler = FrameScheduler::new(layout);

    if options.ready {
        device.send_software_ready(u16::from(options.display_index))?;
        println!("Sent software ready.");
    }
    if options.power_on {
        device.set_monitor_power(u16::from(options.display_index), true)?;
        println!("Sent monitor power on.");
    }

    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(options.fps.max(1)));
    let started = Instant::now();

    for frame_index in 0..options.frames {
        let frame_started = Instant::now();
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

        device.send_jpeg_frame(&packet, options.max_packet_size)?;

        println!(
            "frame={} cmd=0x{:08x} fb=0x{:08x} reset={} payload={} chunks={}",
            frame_index,
            addresses.cmd_addr,
            addresses.fb_addr,
            addresses.reset_jpeg,
            packet.payload.len(),
            packet.bulk_chunks(options.max_packet_size).len()
        );

        let elapsed = frame_started.elapsed();
        if elapsed < frame_interval {
            thread::sleep(frame_interval - elapsed);
        }
    }

    println!(
        "Sent {} frames in {:.3}s",
        options.frames,
        started.elapsed().as_secs_f64()
    );

    Ok(())
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut jpeg_path = None;
    let mut display_index = 1;
    let mut width = None;
    let mut height = None;
    let mut frames = 60;
    let mut fps = 10;
    let mut ready = false;
    let mut power_on = false;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--jpeg" => jpeg_path = Some(PathBuf::from(next_value(&mut args, "--jpeg")?)),
            "--display" => display_index = next_value(&mut args, "--display")?.parse()?,
            "--width" => width = Some(next_value(&mut args, "--width")?.parse()?),
            "--height" => height = Some(next_value(&mut args, "--height")?.parse()?),
            "--frames" => frames = next_value(&mut args, "--frames")?.parse()?,
            "--fps" => fps = next_value(&mut args, "--fps")?.parse()?,
            "--ready" => ready = true,
            "--power-on" => power_on = true,
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }

    Ok(Options {
        jpeg_path: jpeg_path.ok_or("--jpeg is required")?,
        display_index,
        width: width.ok_or("--width is required")?,
        height: height.ok_or("--height is required")?,
        frames,
        fps,
        ready,
        power_on,
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
        "Usage: t6-stream-test --jpeg image.jpg --width 1920 --height 1080 [options]\n\
\n\
Options:\n\
    --display 0|1       Display index, default 1\n\
    --frames N          Number of frames, default 60\n\
    --fps N             Send rate, default 10\n\
    --ready             Send software-ready before stream\n\
    --power-on          Send monitor power-on before stream\n\
    --max-packet N      Bulk fragment size, default 0x19000"
    );
}
