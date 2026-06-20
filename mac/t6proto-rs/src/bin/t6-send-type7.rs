use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use image::imageops::FilterType;
use t6proto::usb::T6Device;
use t6proto::{DEFAULT_MAX_BULK_PACKET_SIZE, Type7JpegTilePacket};
use turbojpeg::Subsamp;

#[derive(Clone, Debug)]
struct Options {
    input_path: PathBuf,
    input_kind: InputKind,
    width: u16,
    height: u16,
    payload_addr: u32,
    sequence: u32,
    canvas_width: u16,
    canvas_height: u16,
    start_addr: u32,
    end_addr: u32,
    ready: bool,
    power_on: bool,
    reset_jpeg_engine: bool,
    dry_run: bool,
    max_packet_size: u32,
    wait_interrupt_ms: u64,
    dump_interrupts: u32,
    dump_header: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    Jpeg,
    Image,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let jpeg = if options.input_kind == InputKind::Image {
        encode_image_to_jpeg_444(&options.input_path, options.width, options.height)?
    } else {
        fs::read(&options.input_path)?
    };
    let packet = Type7JpegTilePacket::new(
        &jpeg,
        options.payload_addr,
        options.sequence,
        options.width,
        options.height,
        options.canvas_width,
        options.canvas_height,
        options.start_addr,
        options.end_addr,
    );
    let chunks = packet.bulk_chunks(options.max_packet_size);

    println!(
        "Sending type7 jpeg tile size={}x{} jpeg_bytes={} payload_bytes={} chunks={} payload_addr=0x{:08x} seq=0x{:08x} canvas={}x{} start=0x{:08x} end=0x{:08x}",
        options.width,
        options.height,
        jpeg.len(),
        packet.payload.len(),
        chunks.len(),
        options.payload_addr,
        options.sequence,
        options.canvas_width,
        options.canvas_height,
        options.start_addr,
        options.end_addr,
    );
    if options.dump_header {
        let header_len = t6proto::TYPE7_JPEG_TILE_HEADER_SIZE;
        println!("type7_header={}", hex_bytes(&packet.payload[..header_len]));
    }

    if options.dry_run {
        println!("Dry run; type7 tile was not sent.");
        return Ok(());
    }

    let device = T6Device::open_first()?;
    if options.ready {
        device.send_software_ready(1)?;
        println!("Sent software ready.");
    }
    if options.power_on {
        device.set_monitor_power(1, true)?;
        println!("Sent monitor power on.");
    }
    if options.reset_jpeg_engine {
        device.reset_jpeg_engine(1)?;
        println!("Sent JPEG engine reset.");
    }

    for (index, chunk) in chunks.iter().enumerate() {
        if let Err(error) = device.write_display_bulk(&chunk.header.to_bytes()) {
            if options.wait_interrupt_ms > 0 {
                let _ = wait_for_interrupts(
                    &device,
                    options.sequence,
                    Duration::from_millis(options.wait_interrupt_ms),
                    options.dump_interrupts,
                );
            }
            return Err(format!(
                "type7 bulk header error at chunk {}/{} offset={} size={}: {error}",
                index + 1,
                chunks.len(),
                chunk.header.packet_offset,
                chunk.header.packet_size
            )
            .into());
        }
        if let Err(error) = device.write_display_bulk(chunk.data) {
            if options.wait_interrupt_ms > 0 {
                let _ = wait_for_interrupts(
                    &device,
                    options.sequence,
                    Duration::from_millis(options.wait_interrupt_ms),
                    options.dump_interrupts,
                );
            }
            return Err(format!(
                "type7 bulk data error at chunk {}/{} offset={} size={}: {error}",
                index + 1,
                chunks.len(),
                chunk.header.packet_offset,
                chunk.header.packet_size
            )
            .into());
        }
        println!(
            "sent chunk {}/{} offset={} size={}",
            index + 1,
            chunks.len(),
            chunk.header.packet_offset,
            chunk.header.packet_size
        );
    }

    if options.wait_interrupt_ms > 0 {
        wait_for_interrupts(
            &device,
            options.sequence,
            Duration::from_millis(options.wait_interrupt_ms),
            options.dump_interrupts,
        )?;
    }

    Ok(())
}

fn wait_for_interrupts(
    device: &T6Device,
    target: u32,
    duration: Duration,
    mut dumps: u32,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + duration;
    let mut packets = 0u32;
    let mut fences = 0u32;
    let mut matched = 0u32;
    let mut last_data = 0u32;
    let mut last_event = 0u8;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let timeout = (deadline - now).min(Duration::from_millis(10));
        match device.read_interrupt_packet_timeout(timeout) {
            Ok(packet) => {
                if dumps > 0 {
                    println!("interrupt_raw={}", hex_bytes(&packet));
                    dumps -= 1;
                }
                let interrupt = t6proto::DisplayInterrupt::parse(&packet);
                packets = packets.saturating_add(1);
                last_data = interrupt.display_data;
                last_event = interrupt.display_event;
                if interrupt.has_fence_id {
                    fences = fences.saturating_add(1);
                    if interrupt.display_data == target {
                        matched = matched.saturating_add(1);
                        break;
                    }
                }
            }
            Err(rusb::Error::Timeout) => break,
            Err(error) => return Err(format!("interrupt read error: {error}").into()),
        }
    }

    println!(
        "interrupts={} fences={} matched={} target_data=0x{:08x} ack_lag={} last_event=0x{:02x} last_data=0x{:08x}",
        packets,
        fences,
        matched,
        target,
        target.saturating_sub(last_data),
        last_event,
        last_data,
    );

    Ok(())
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut jpeg_path = None;
    let mut image_path = None;
    let mut width = 64;
    let mut height = 1080;
    let mut payload_addr = 0x02d0_0000;
    let mut sequence = 1;
    let mut canvas_width = 1920;
    let mut canvas_height = 1920;
    let mut start_addr = 0x30;
    let mut end_addr = 0x1fe030;
    let mut ready = false;
    let mut power_on = false;
    let mut reset_jpeg_engine = false;
    let mut dry_run = false;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut wait_interrupt_ms = 0;
    let mut dump_interrupts = 0;
    let mut dump_header = false;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--jpeg" => jpeg_path = Some(PathBuf::from(next_value(&mut args, "--jpeg")?)),
            "--image" => image_path = Some(PathBuf::from(next_value(&mut args, "--image")?)),
            "--width" => width = next_value(&mut args, "--width")?.parse()?,
            "--height" => height = next_value(&mut args, "--height")?.parse()?,
            "--payload-addr" => {
                payload_addr = parse_u32(&next_value(&mut args, "--payload-addr")?)?
            }
            "--sequence" => sequence = parse_u32(&next_value(&mut args, "--sequence")?)?,
            "--canvas-width" => canvas_width = next_value(&mut args, "--canvas-width")?.parse()?,
            "--canvas-height" => {
                canvas_height = next_value(&mut args, "--canvas-height")?.parse()?
            }
            "--start-addr" => start_addr = parse_u32(&next_value(&mut args, "--start-addr")?)?,
            "--end-addr" => end_addr = parse_u32(&next_value(&mut args, "--end-addr")?)?,
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "--wait-interrupt-ms" => {
                wait_interrupt_ms = next_value(&mut args, "--wait-interrupt-ms")?.parse()?
            }
            "--dump-interrupts" => {
                dump_interrupts = next_value(&mut args, "--dump-interrupts")?.parse()?
            }
            "--dump-header" => dump_header = true,
            "--ready" => ready = true,
            "--power-on" => power_on = true,
            "--reset-jpeg-engine" => reset_jpeg_engine = true,
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }

    let (input_path, input_kind) = match (jpeg_path, image_path) {
        (Some(jpeg_path), None) => (jpeg_path, InputKind::Jpeg),
        (None, Some(image_path)) => (image_path, InputKind::Image),
        (None, None) => return Err("--jpeg or --image is required".into()),
        (Some(_), Some(_)) => return Err("use only one of --jpeg or --image".into()),
    };

    Ok(Options {
        input_path,
        input_kind,
        width,
        height,
        payload_addr,
        sequence,
        canvas_width,
        canvas_height,
        start_addr,
        end_addr,
        ready,
        power_on,
        reset_jpeg_engine,
        dry_run,
        max_packet_size,
        wait_interrupt_ms,
        dump_interrupts,
        dump_header,
    })
}

fn encode_image_to_jpeg_444(
    image_path: &PathBuf,
    width: u16,
    height: u16,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let rgb = image::open(image_path)?
        .resize_exact(u32::from(width), u32::from(height), FilterType::Lanczos3)
        .to_rgb8();
    let jpeg = turbojpeg::compress_image(&rgb, 90, Subsamp::None)?;
    Ok(jpeg.to_vec())
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("{name} requires a value").into())
}

fn parse_u32(value: &str) -> Result<u32, Box<dyn Error>> {
    Ok(
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            u32::from_str_radix(hex, 16)?
        } else {
            value.parse()?
        },
    )
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn print_help() {
    println!(
        "Usage: t6-send-type7 (--jpeg tile.jpg | --image image.png) [options]\n\
Options:\n\
    --width N               Tile width, default 64\n\
    --height N              Tile height, default 1080\n\
    --payload-addr N        Bulk payload address, default 0x02d00000\n\
    --sequence N            Type7 sequence/fence id, default 1\n\
    --canvas-width N        Type7 canvas width, default 1920\n\
    --canvas-height N       Type7 canvas height, default 1920\n\
    --start-addr N          Type7 start address, default 0x30\n\
    --end-addr N            Type7 end address, default 0x1fe030\n\
    --max-packet N          Bulk fragment size, default 0x19000\n\
    --wait-interrupt-ms N   Read interrupts after send for up to N ms\n\
    --dump-interrupts N     Print first N raw interrupt packets\n\
    --dump-header           Print the 48-byte type7 video header before sending\n\
    --ready                 Send software-ready before tile\n\
    --power-on              Send monitor power-on before tile\n\
    --reset-jpeg-engine     Send vendor JPEG engine reset before tile\n\
    --dry-run               Build packet but do not open USB or send"
    );
}
