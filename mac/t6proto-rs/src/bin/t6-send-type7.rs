use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use image::RgbImage;
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
    crop_x: Option<u32>,
    crop_y: Option<u32>,
    quality: i32,
    subsamp: Subsamp,
    ready: bool,
    power_on: bool,
    reset_jpeg_engine: bool,
    dry_run: bool,
    max_packet_size: u32,
    wait_interrupt_ms: u64,
    dump_interrupts: u32,
    dump_header: bool,
    scan_known_addresses: bool,
    scan_sleep_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    Jpeg,
    Image,
    SolidWhite,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let jpeg = build_jpeg(&options)?;
    let jpeg_info = parse_jpeg_info(&jpeg)?;
    println!(
        "JPEG info: marker=0x{:02x} progressive={} components={} sampling={}",
        jpeg_info.sof_marker,
        jpeg_info.is_progressive,
        jpeg_info.components.len(),
        jpeg_info.sampling_summary()
    );

    let address_pairs = if options.scan_known_addresses {
        known_address_pairs().to_vec()
    } else {
        vec![(options.start_addr, options.end_addr)]
    };
    let mut payload_addr = options.payload_addr;
    let mut sequence = options.sequence;

    if options.dry_run {
        for (index, &(start_addr, end_addr)) in address_pairs.iter().enumerate() {
            let packet = make_packet(
                &options,
                &jpeg,
                payload_addr,
                sequence,
                start_addr,
                end_addr,
            );
            print_packet_summary(
                &options,
                &packet,
                index + 1,
                address_pairs.len(),
                sequence,
                start_addr,
                end_addr,
            );
            payload_addr = next_ring_addr(payload_addr, packet.payload.len());
            sequence = sequence.wrapping_add(1);
        }
        println!("Dry run; type7 tiles were not sent.");
        return Ok(());
    }

    let device = T6Device::open_first().map_err(|error| format!("open device failed: {error}"))?;
    if options.ready {
        device
            .send_software_ready(1)
            .map_err(|error| format!("send software-ready failed: {error}"))?;
        println!("Sent software ready.");
    }
    if options.power_on {
        device
            .set_monitor_power(1, true)
            .map_err(|error| format!("send monitor power-on failed: {error}"))?;
        println!("Sent monitor power on.");
    }
    if options.reset_jpeg_engine {
        device
            .reset_jpeg_engine(1)
            .map_err(|error| format!("send JPEG engine reset failed: {error}"))?;
        println!("Sent JPEG engine reset.");
    }

    for (index, &(start_addr, end_addr)) in address_pairs.iter().enumerate() {
        let packet = make_packet(
            &options,
            &jpeg,
            payload_addr,
            sequence,
            start_addr,
            end_addr,
        );
        print_packet_summary(
            &options,
            &packet,
            index + 1,
            address_pairs.len(),
            sequence,
            start_addr,
            end_addr,
        );
        send_packet(&device, &options, &packet, sequence)?;
        payload_addr = next_ring_addr(payload_addr, packet.payload.len());
        sequence = sequence.wrapping_add(1);

        if options.scan_known_addresses && index + 1 < address_pairs.len() {
            thread::sleep(Duration::from_millis(options.scan_sleep_ms));
        }
    }

    Ok(())
}

fn build_jpeg(options: &Options) -> Result<Vec<u8>, Box<dyn Error>> {
    match options.input_kind {
        InputKind::Image => encode_image_to_jpeg(
            &options.input_path,
            options.width,
            options.height,
            options.crop_x,
            options.crop_y,
            options.quality,
            options.subsamp,
        ),
        InputKind::Jpeg => Ok(fs::read(&options.input_path)?),
        InputKind::SolidWhite => encode_solid_white_to_jpeg(
            options.width,
            options.height,
            options.quality,
            options.subsamp,
        ),
    }
}

fn make_packet(
    options: &Options,
    jpeg: &[u8],
    payload_addr: u32,
    sequence: u32,
    start_addr: u32,
    end_addr: u32,
) -> Type7JpegTilePacket {
    Type7JpegTilePacket::new(
        jpeg,
        payload_addr,
        sequence,
        options.width,
        options.height,
        options.canvas_width,
        options.canvas_height,
        start_addr,
        end_addr,
    )
}

fn print_packet_summary(
    options: &Options,
    packet: &Type7JpegTilePacket,
    scan_index: usize,
    scan_count: usize,
    sequence: u32,
    start_addr: u32,
    end_addr: u32,
) {
    let chunks = packet.bulk_chunks(options.max_packet_size);
    println!(
        "Sending type7 jpeg tile {}/{} size={}x{} jpeg_bytes={} payload_bytes={} chunks={} payload_addr=0x{:08x} seq=0x{:08x} canvas={}x{} start=0x{:08x} end=0x{:08x}",
        scan_index,
        scan_count,
        options.width,
        options.height,
        packet
            .payload
            .len()
            .saturating_sub(t6proto::TYPE7_JPEG_TILE_HEADER_SIZE),
        packet.payload.len(),
        chunks.len(),
        packet.payload_address,
        sequence,
        options.canvas_width,
        options.canvas_height,
        start_addr,
        end_addr,
    );
    if options.dump_header {
        let header_len = t6proto::TYPE7_JPEG_TILE_HEADER_SIZE;
        println!("type7_header={}", hex_bytes(&packet.payload[..header_len]));
    }
}

fn send_packet(
    device: &T6Device,
    options: &Options,
    packet: &Type7JpegTilePacket,
    sequence: u32,
) -> Result<(), Box<dyn Error>> {
    let chunks = packet.bulk_chunks(options.max_packet_size);
    for (index, chunk) in chunks.iter().enumerate() {
        if let Err(error) = device.write_display_bulk(&chunk.header.to_bytes()) {
            if options.wait_interrupt_ms > 0 {
                let _ = wait_for_interrupts(
                    device,
                    sequence,
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
                    device,
                    sequence,
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
            device,
            sequence,
            Duration::from_millis(options.wait_interrupt_ms),
            options.dump_interrupts,
        )?;
    }

    Ok(())
}

fn next_ring_addr(payload_addr: u32, payload_len: usize) -> u32 {
    let aligned = ((payload_len as u32).saturating_add(0x3ff)) & !0x3ff;
    payload_addr.wrapping_add(aligned.saturating_sub(32))
}

fn known_address_pairs() -> &'static [(u32, u32)] {
    &[
        (0x0000_0030, 0x001f_e030),
        (0x018a_aaf0, 0x01aa_8af0),
        (0x018a_b210, 0x01aa_9210),
        (0x018a_ab10, 0x01aa_8b10),
        (0x018c_8af0, 0x01ab_7af0),
        (0x00ca_fcd0, 0x00e8_0cd0),
        (0x0193_2190, 0x01ae_c990),
        (0x00c5_5590, 0x00e5_3590),
        (0x00d4_5cb0, 0x00ec_bcb0),
    ]
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
    let mut crop_x = None;
    let mut crop_y = None;
    let mut quality = 90;
    let mut subsamp = Subsamp::None;
    let mut ready = false;
    let mut power_on = false;
    let mut reset_jpeg_engine = false;
    let mut dry_run = false;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut wait_interrupt_ms = 0;
    let mut dump_interrupts = 0;
    let mut dump_header = false;
    let mut solid_white = false;
    let mut scan_known_addresses = false;
    let mut scan_sleep_ms = 100;
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
            "--crop-x" => crop_x = Some(next_value(&mut args, "--crop-x")?.parse()?),
            "--crop-y" => crop_y = Some(next_value(&mut args, "--crop-y")?.parse()?),
            "--quality" => quality = next_value(&mut args, "--quality")?.parse()?,
            "--subsamp" => subsamp = parse_subsampling(&next_value(&mut args, "--subsamp")?)?,
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "--wait-interrupt-ms" => {
                wait_interrupt_ms = next_value(&mut args, "--wait-interrupt-ms")?.parse()?
            }
            "--dump-interrupts" => {
                dump_interrupts = next_value(&mut args, "--dump-interrupts")?.parse()?
            }
            "--dump-header" => dump_header = true,
            "--solid-white" => solid_white = true,
            "--scan-known-addresses" => scan_known_addresses = true,
            "--scan-sleep-ms" => {
                scan_sleep_ms = next_value(&mut args, "--scan-sleep-ms")?.parse()?
            }
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

    let (input_path, input_kind) = match (jpeg_path, image_path, solid_white) {
        (Some(jpeg_path), None, false) => (jpeg_path, InputKind::Jpeg),
        (None, Some(image_path), false) => (image_path, InputKind::Image),
        (None, None, true) => (PathBuf::new(), InputKind::SolidWhite),
        (None, None, false) => return Err("--jpeg, --image, or --solid-white is required".into()),
        _ => return Err("use only one of --jpeg, --image, or --solid-white".into()),
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
        crop_x,
        crop_y,
        quality,
        subsamp,
        ready,
        power_on,
        reset_jpeg_engine,
        dry_run,
        max_packet_size,
        wait_interrupt_ms,
        dump_interrupts,
        dump_header,
        scan_known_addresses,
        scan_sleep_ms,
    })
}

fn encode_image_to_jpeg(
    image_path: &PathBuf,
    width: u16,
    height: u16,
    crop_x: Option<u32>,
    crop_y: Option<u32>,
    quality: i32,
    subsamp: Subsamp,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let image = image::open(image_path)?;
    let rgb = match (crop_x, crop_y) {
        (Some(x), Some(y)) => image
            .crop_imm(x, y, u32::from(width), u32::from(height))
            .to_rgb8(),
        (None, None) => image
            .resize_exact(u32::from(width), u32::from(height), FilterType::Lanczos3)
            .to_rgb8(),
        _ => return Err("--crop-x and --crop-y must be specified together".into()),
    };
    let jpeg = turbojpeg::compress_image(&rgb, quality, subsamp)?;
    Ok(jpeg.to_vec())
}

fn encode_solid_white_to_jpeg(
    width: u16,
    height: u16,
    quality: i32,
    subsamp: Subsamp,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let rgb = RgbImage::from_pixel(
        u32::from(width),
        u32::from(height),
        image::Rgb([255, 255, 255]),
    );
    let jpeg = turbojpeg::compress_image(&rgb, quality, subsamp)?;
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

fn parse_subsampling(value: &str) -> Result<Subsamp, Box<dyn Error>> {
    match value {
        "420" => Ok(Subsamp::Sub2x2),
        "422" => Ok(Subsamp::Sub2x1),
        "444" => Ok(Subsamp::None),
        _ => Err("--subsamp must be one of 420, 422, 444".into()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JpegInfo {
    width: u16,
    height: u16,
    sof_marker: u8,
    is_progressive: bool,
    components: Vec<JpegComponent>,
}

impl JpegInfo {
    fn sampling_summary(&self) -> String {
        self.components
            .iter()
            .map(|component| {
                format!(
                    "id{}:{}x{}",
                    component.id, component.h_sampling, component.v_sampling
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JpegComponent {
    id: u8,
    h_sampling: u8,
    v_sampling: u8,
}

fn parse_jpeg_info(jpeg: &[u8]) -> Result<JpegInfo, Box<dyn Error>> {
    if jpeg.len() < 4 || jpeg[0] != 0xff || jpeg[1] != 0xd8 {
        return Err("input is not a JPEG file".into());
    }

    let mut index = 2;
    while index + 4 <= jpeg.len() {
        while index < jpeg.len() && jpeg[index] == 0xff {
            index += 1;
        }
        if index >= jpeg.len() {
            break;
        }

        let marker = jpeg[index];
        index += 1;

        if marker == 0xd8 || marker == 0xd9 {
            continue;
        }
        if index + 2 > jpeg.len() {
            break;
        }

        let segment_len = u16::from_be_bytes([jpeg[index], jpeg[index + 1]]) as usize;
        if segment_len < 2 || index + segment_len > jpeg.len() {
            break;
        }

        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if segment_len < 7 {
                break;
            }
            let height = u16::from_be_bytes([jpeg[index + 3], jpeg[index + 4]]);
            let width = u16::from_be_bytes([jpeg[index + 5], jpeg[index + 6]]);
            let component_count = jpeg[index + 7] as usize;
            let mut components = Vec::with_capacity(component_count);
            let mut component_index = index + 8;

            for _ in 0..component_count {
                if component_index + 3 > index + segment_len {
                    return Err("invalid JPEG SOF component table".into());
                }
                let sampling = jpeg[component_index + 1];
                components.push(JpegComponent {
                    id: jpeg[component_index],
                    h_sampling: sampling >> 4,
                    v_sampling: sampling & 0x0f,
                });
                component_index += 3;
            }

            return Ok(JpegInfo {
                width,
                height,
                sof_marker: marker,
                is_progressive: matches!(marker, 0xc2 | 0xc6 | 0xca | 0xce),
                components,
            });
        }

        index += segment_len;
    }

    Err("could not find JPEG dimensions".into())
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
        "Usage: t6-send-type7 (--jpeg tile.jpg | --image image.png | --solid-white) [options]\n\
Options:\n\
    --width N               Tile width, default 64\n\
    --height N              Tile height, default 1080\n\
    --payload-addr N        Bulk payload address, default 0x02d00000\n\
    --sequence N            Type7 sequence/fence id, default 1\n\
    --canvas-width N        Type7 canvas width, default 1920\n\
    --canvas-height N       Type7 canvas height, default 1920\n\
    --start-addr N          Type7 start address, default 0x30\n\
    --end-addr N            Type7 end address, default 0x1fe030\n\
    --crop-x N              Crop source image at x instead of resizing whole image\n\
    --crop-y N              Crop source image at y instead of resizing whole image\n\
    --quality N             JPEG quality for --image, default 90\n\
    --subsamp 420|422|444   JPEG subsampling for --image, default 444\n\
    --max-packet N          Bulk fragment size, default 0x19000\n\
    --wait-interrupt-ms N   Read interrupts after send for up to N ms\n\
    --dump-interrupts N     Print first N raw interrupt packets\n\
    --dump-header           Print the 48-byte type7 video header before sending\n\
    --solid-white           Generate a solid white JPEG tile instead of reading an input file\n\
    --scan-known-addresses  Send the tile to known Windows-captured start/end address pairs\n\
    --scan-sleep-ms N       Sleep between scanned address pairs, default 100\n\
    --ready                 Send software-ready before tile\n\
    --power-on              Send monitor power-on before tile\n\
    --reset-jpeg-engine     Send vendor JPEG engine reset before tile\n\
    --dry-run               Build packet but do not open USB or send"
    );
}
