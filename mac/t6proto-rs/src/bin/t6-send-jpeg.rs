use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use image::imageops::FilterType;
use t6proto::usb::T6Device;
use t6proto::{
    DEFAULT_MAX_BULK_PACKET_SIZE, JPEG_PADDING_SIZE, JpegFramePacket, VIDEO_COLOR_NV12,
    VIDEO_COLOR_YV12, VIDEO_FLAG_RESET_JPEG, VramLayout,
};
use turbojpeg::Subsamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutKind {
    OnePort1080p,
    TwoPort1080pSecondary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetFormat {
    Nv12,
    Yv12,
}

#[derive(Clone, Debug)]
struct Options {
    input_path: PathBuf,
    input_kind: InputKind,
    display_index: u8,
    width: Option<u16>,
    height: Option<u16>,
    layout: LayoutKind,
    cmd_addr: Option<u32>,
    fb_addr: Option<u32>,
    target_format: TargetFormat,
    reset_jpeg: bool,
    ready: bool,
    power_on: bool,
    dry_run: bool,
    ram_size_mb: Option<u8>,
    max_packet_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    Jpeg,
    Image,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let jpeg = if options.input_kind == InputKind::Image {
        encode_image_to_jpeg_420(&options.input_path, options.width, options.height)?
    } else {
        fs::read(&options.input_path)?
    };
    let jpeg_info = parse_jpeg_info(&jpeg)?;
    let width = options.width.unwrap_or(jpeg_info.width);
    let height = options.height.unwrap_or(jpeg_info.height);

    let (device, ram_size_mb) = if options.dry_run {
        (None, options.ram_size_mb.unwrap_or(58))
    } else {
        let device = T6Device::open_first()?;
        let ram_size_mb = device.read_video_ram_size_mb()?;
        (Some(device), ram_size_mb)
    };
    let layout = match options.layout {
        LayoutKind::OnePort1080p => VramLayout::one_port_1080p(ram_size_mb),
        LayoutKind::TwoPort1080pSecondary => VramLayout::two_port_1080p_secondary(ram_size_mb),
    };
    let cmd_addr = options.cmd_addr.unwrap_or(layout.cmd_addr);
    let fb_addr = options.fb_addr.unwrap_or(layout.fb_addrs[1]);
    let flags = if options.reset_jpeg {
        VIDEO_FLAG_RESET_JPEG
    } else {
        0
    };

    println!(
        "Sending JPEG display={} size={}x{} jpeg_bytes={} ram={}MB cmd=0x{:08x} fb=0x{:08x} padding={} reset_flag={}",
        options.display_index,
        width,
        height,
        jpeg.len(),
        ram_size_mb,
        cmd_addr,
        fb_addr,
        JPEG_PADDING_SIZE,
        flags != 0
    );
    println!(
        "JPEG info: marker=0x{:02x} progressive={} components={} sampling={}",
        jpeg_info.sof_marker,
        jpeg_info.is_progressive,
        jpeg_info.components.len(),
        jpeg_info.sampling_summary()
    );
    if !jpeg_info.looks_like_baseline_ycbcr_420() {
        println!(
            "Warning: JPEG is not baseline 3-component 4:2:0. T6 may decode color incorrectly."
        );
    }

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

    let packet = JpegFramePacket::new_with_target_format(
        options.display_index,
        &jpeg,
        width,
        height,
        cmd_addr,
        fb_addr,
        options.target_format.video_color(),
        flags,
    );
    println!(
        "Bulk payload bytes={} chunks={}",
        packet.payload.len(),
        packet.bulk_chunks(options.max_packet_size).len()
    );

    if options.dry_run {
        println!("Dry run; frame was not sent.");
    } else {
        device
            .unwrap()
            .send_jpeg_frame(&packet, options.max_packet_size)?;
        println!("Done.");
    }

    Ok(())
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut jpeg_path = None;
    let mut image_path = None;
    let mut display_index = 1;
    let mut width = None;
    let mut height = None;
    let mut layout = LayoutKind::TwoPort1080pSecondary;
    let mut cmd_addr = None;
    let mut fb_addr = None;
    let mut target_format = TargetFormat::Nv12;
    let mut reset_jpeg = true;
    let mut ready = false;
    let mut power_on = false;
    let mut dry_run = false;
    let mut ram_size_mb = None;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--jpeg" => jpeg_path = Some(PathBuf::from(next_value(&mut args, "--jpeg")?)),
            "--image" => image_path = Some(PathBuf::from(next_value(&mut args, "--image")?)),
            "--display" => display_index = next_value(&mut args, "--display")?.parse()?,
            "--width" => width = Some(next_value(&mut args, "--width")?.parse()?),
            "--height" => height = Some(next_value(&mut args, "--height")?.parse()?),
            "--layout" => {
                layout = match next_value(&mut args, "--layout")?.as_str() {
                    "one-port-1080p" => LayoutKind::OnePort1080p,
                    "two-port-1080p-secondary" => LayoutKind::TwoPort1080pSecondary,
                    value => return Err(format!("unknown layout: {value}").into()),
                };
            }
            "--cmd-addr" => cmd_addr = Some(parse_u32(&next_value(&mut args, "--cmd-addr")?)?),
            "--fb-addr" => fb_addr = Some(parse_u32(&next_value(&mut args, "--fb-addr")?)?),
            "--target-format" => {
                target_format = match next_value(&mut args, "--target-format")?.as_str() {
                    "nv12" => TargetFormat::Nv12,
                    "yv12" => TargetFormat::Yv12,
                    value => return Err(format!("unknown target format: {value}").into()),
                };
            }
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "--no-reset-jpeg" => reset_jpeg = false,
            "--ready" => ready = true,
            "--power-on" => power_on = true,
            "--dry-run" => dry_run = true,
            "--ram-size-mb" => ram_size_mb = Some(next_value(&mut args, "--ram-size-mb")?.parse()?),
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
        display_index,
        width,
        height,
        layout,
        cmd_addr,
        fb_addr,
        target_format,
        reset_jpeg,
        ready,
        power_on,
        dry_run,
        ram_size_mb,
        max_packet_size,
    })
}

fn encode_image_to_jpeg_420(
    image_path: &PathBuf,
    width: Option<u16>,
    height: Option<u16>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let image = image::open(image_path)?;
    let rgb = match (width, height) {
        (Some(width), Some(height)) => image
            .resize_exact(u32::from(width), u32::from(height), FilterType::Lanczos3)
            .to_rgb8(),
        (None, None) => image.to_rgb8(),
        _ => return Err("--width and --height must be specified together".into()),
    };

    let jpeg = turbojpeg::compress_image(&rgb, 95, Subsamp::Sub2x2)?;
    Ok(jpeg.to_vec())
}

impl TargetFormat {
    fn video_color(self) -> u32 {
        match self {
            Self::Nv12 => VIDEO_COLOR_NV12,
            Self::Yv12 => VIDEO_COLOR_YV12,
        }
    }
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

    fn looks_like_baseline_ycbcr_420(&self) -> bool {
        self.sof_marker == 0xc0
            && self.components.len() == 3
            && self.components[0].h_sampling == 2
            && self.components[0].v_sampling == 2
            && self.components[1].h_sampling == 1
            && self.components[1].v_sampling == 1
            && self.components[2].h_sampling == 1
            && self.components[2].v_sampling == 1
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

fn print_help() {
    println!(
        "Usage: t6-send-jpeg (--jpeg image.jpg | --image image.png) [options]\n\
\n\
Options:\n\
    --jpeg PATH                         Send existing baseline 4:2:0 JPEG\n\
    --image PATH                        Convert PNG/etc to baseline 4:2:0 JPEG before sending\n\
    --display 0|1                       Display index, default 1\n\
    --layout one-port-1080p|two-port-1080p-secondary\n\
                                        VRAM layout, default two-port-1080p-secondary\n\
    --width N --height N                Override JPEG dimensions sent to T6\n\
    --cmd-addr 0xNNNNNNNN               Override command/payload VRAM address\n\
    --fb-addr 0xNNNNNNNN                Override framebuffer VRAM address\n\
    --target-format nv12|yv12           Target format after JPEG decode, default nv12\n\
    --max-packet N                      Bulk fragment size, default 0x19000\n\
    --no-reset-jpeg                     Clear VIDEO_FLIP_HEADER reset flag\n\
    --ready                             Send software-ready before frame\n\
    --power-on                          Send monitor power-on before frame\n\
    --dry-run                           Build packet but do not open USB or send\n\
    --ram-size-mb N                     RAM size for dry-run address planning, default 58"
    );
}
