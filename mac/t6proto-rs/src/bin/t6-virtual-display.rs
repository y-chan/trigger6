use std::env;
use std::error::Error;
use std::ffi::CStr;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use image::RgbImage;
use t6proto::usb::T6Device;
use t6proto::{
    BulkDmaHeader, BulkTransferChunk, DEFAULT_MAX_BULK_PACKET_SIZE, FrameScheduler,
    JpegFramePacket, RawFramePacket, VIDEO_COLOR_NV12, VIDEO_COLOR_YUV444, VIDEO_COLOR_YV12,
    VIDEO_FLAG_RESET_JPEG, VramLayout,
};
use turbojpeg::Subsamp;

type FrameCallback = extern "C" fn(
    u32,
    *const u8,
    usize,
    usize,
    usize,
    usize,
    *const u8,
    usize,
    usize,
    usize,
    usize,
    *mut c_void,
);

const PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
const PIXEL_FORMAT_420F: u32 = u32::from_be_bytes(*b"420f");
const PIXEL_FORMAT_420V: u32 = u32::from_be_bytes(*b"420v");

unsafe extern "C" {
    fn t6_vd_start(
        width: usize,
        height: usize,
        refresh_rate: f64,
        pixel_format: u32,
        callback: FrameCallback,
        user_data: *mut c_void,
    ) -> u32;
    fn t6_vd_stop();
    fn t6_vd_last_error() -> *const std::ffi::c_char;
}

#[derive(Clone, Debug)]
struct Options {
    display_index: u8,
    width: u16,
    height: u16,
    rotate: Rotation,
    fps: u32,
    frames: Option<u32>,
    quality: i32,
    subsamp: JpegSubsampling,
    jpeg_target: JpegTarget,
    chroma_mode: ChromaMode,
    yuv_matrix: YuvMatrix,
    yuv_range: YuvRange,
    transport: Transport,
    capture_format: CaptureFormat,
    raw_bulk_mode: RawBulkMode,
    ready: bool,
    power_on: bool,
    profile: bool,
    dry_run: bool,
    ram_size_mb: Option<u8>,
    usb_timeout_ms: u64,
    max_packet_size: u32,
    dump_first_frame: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rotation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JpegSubsampling {
    Yuv420,
    Yuv422,
    Yuv444,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transport {
    Jpeg,
    Nv12,
    Rgb24,
    Yv12,
    Yuv444,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JpegTarget {
    Nv12,
    Yv12,
    Yuv444,
}

impl JpegTarget {
    fn video_color(self) -> u32 {
        match self {
            Self::Nv12 => VIDEO_COLOR_NV12,
            Self::Yv12 => VIDEO_COLOR_YV12,
            Self::Yuv444 => VIDEO_COLOR_YUV444,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChromaMode {
    Average,
    Saturated,
    TopLeft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum YuvMatrix {
    Bt601,
    Bt709,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum YuvRange {
    Full,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RawBulkMode {
    Fragmented,
    Single,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureFormat {
    Bgra,
    Nv12FullRange,
    Nv12VideoRange,
}

impl CaptureFormat {
    fn pixel_format(self) -> u32 {
        match self {
            Self::Bgra => PIXEL_FORMAT_BGRA,
            Self::Nv12FullRange => PIXEL_FORMAT_420F,
            Self::Nv12VideoRange => PIXEL_FORMAT_420V,
        }
    }
}

impl JpegSubsampling {
    fn turbojpeg(self) -> Subsamp {
        match self {
            Self::Yuv420 => Subsamp::Sub2x2,
            Self::Yuv422 => Subsamp::Sub2x1,
            Self::Yuv444 => Subsamp::None,
        }
    }
}

impl Rotation {
    fn output_size(self, width: u16, height: u16) -> (u16, u16) {
        match self {
            Self::Deg0 | Self::Deg180 => (width, height),
            Self::Deg90 | Self::Deg270 => (height, width),
        }
    }
}

struct SenderState {
    options: Options,
    device: Option<T6Device>,
    scheduler: FrameScheduler,
    frame_interval: Duration,
    next_send_at: Instant,
    started_at: Instant,
    sent_frames: u32,
    dropped_frames: u32,
    profile_stats: ProfileStats,
    first_frame_dumped: bool,
    sending: AtomicBool,
    stopped: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProfileStats {
    frames: u32,
    convert: Duration,
    encode: Duration,
    packet: Duration,
    usb: Duration,
    total: Duration,
}

impl ProfileStats {
    fn push(&mut self, sample: ProfileSample) {
        self.frames += 1;
        self.convert += sample.convert;
        self.encode += sample.encode;
        self.packet += sample.packet;
        self.usb += sample.usb;
        self.total += sample.total;
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    fn summary(self) -> String {
        if self.frames == 0 {
            return "no profile samples".to_string();
        }

        format!(
            "profile frames={} avg_ms convert={:.2} encode={:.2} packet={:.2} usb={:.2} total={:.2}",
            self.frames,
            avg_ms(self.convert, self.frames),
            avg_ms(self.encode, self.frames),
            avg_ms(self.packet, self.frames),
            avg_ms(self.usb, self.frames),
            avg_ms(self.total, self.frames)
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ProfileSample {
    convert: Duration,
    encode: Duration,
    packet: Duration,
    usb: Duration,
    total: Duration,
}

fn avg_ms(duration: Duration, frames: u32) -> f64 {
    duration.as_secs_f64() * 1000.0 / f64::from(frames)
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;

    let (device, ram_size_mb) = if options.dry_run {
        (None, options.ram_size_mb.unwrap_or(58))
    } else {
        let mut device = T6Device::open_first()?;
        device.set_timeout(Duration::from_millis(options.usb_timeout_ms));
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
    let state = Box::new(SenderState {
        frame_interval: Duration::from_secs_f64(1.0 / f64::from(options.fps.max(1))),
        next_send_at: Instant::now(),
        started_at: Instant::now(),
        sent_frames: 0,
        dropped_frames: 0,
        profile_stats: ProfileStats::default(),
        first_frame_dumped: false,
        sending: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        options,
        device,
        scheduler: FrameScheduler::new(layout),
    });
    let state_ptr = Box::into_raw(state);

    let display_id = unsafe {
        t6_vd_start(
            usize::from((*state_ptr).options.width),
            usize::from((*state_ptr).options.height),
            f64::from((*state_ptr).options.fps),
            (*state_ptr).options.capture_format.pixel_format(),
            frame_callback,
            state_ptr.cast(),
        )
    };
    if display_id == 0 {
        let _state = unsafe { Box::from_raw(state_ptr) };
        return Err(format!(
            "failed to create virtual display or display stream: {}",
            unsafe { last_virtual_display_error() }
        )
        .into());
    }

    println!(
        "Started virtual display id={} capture={}x{} output={}x{} rotate={:?} fps={} transport={:?} capture_format={:?} quality={} subsamp={:?} dry_run={}",
        display_id,
        unsafe { (*state_ptr).options.width },
        unsafe { (*state_ptr).options.height },
        unsafe {
            (*state_ptr)
                .options
                .rotate
                .output_size((*state_ptr).options.width, (*state_ptr).options.height)
                .0
        },
        unsafe {
            (*state_ptr)
                .options
                .rotate
                .output_size((*state_ptr).options.width, (*state_ptr).options.height)
                .1
        },
        unsafe { (*state_ptr).options.rotate },
        unsafe { (*state_ptr).options.fps },
        unsafe { (*state_ptr).options.transport },
        unsafe { (*state_ptr).options.capture_format },
        unsafe { (*state_ptr).options.quality },
        unsafe { (*state_ptr).options.subsamp },
        unsafe { (*state_ptr).options.dry_run }
    );
    println!("Stop with Ctrl-C, or use --frames N for a bounded run.");

    while !unsafe { (*state_ptr).stopped.load(Ordering::Relaxed) } {
        thread::sleep(Duration::from_millis(100));
    }

    unsafe {
        t6_vd_stop();
        let state = Box::from_raw(state_ptr);
        println!(
            "Sent {} frames, dropped {} frames in {:.3}s",
            state.sent_frames,
            state.dropped_frames,
            state.started_at.elapsed().as_secs_f64()
        );
    }

    Ok(())
}

unsafe fn last_virtual_display_error() -> String {
    let error = unsafe { t6_vd_last_error() };
    if error.is_null() {
        return "unknown error".to_string();
    }

    unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned()
}

extern "C" fn frame_callback(
    pixel_format: u32,
    plane0: *const u8,
    plane0_byte_count: usize,
    width: usize,
    height: usize,
    plane0_stride: usize,
    plane1: *const u8,
    plane1_byte_count: usize,
    plane1_width: usize,
    plane1_height: usize,
    plane1_stride: usize,
    user_data: *mut c_void,
) {
    if plane0.is_null() || user_data.is_null() {
        return;
    }

    let state = unsafe { &mut *(user_data.cast::<SenderState>()) };
    if state.stopped.load(Ordering::Relaxed) || Instant::now() < state.next_send_at {
        state.dropped_frames = state.dropped_frames.saturating_add(1);
        return;
    }

    if state.sending.swap(true, Ordering::Acquire) {
        state.dropped_frames = state.dropped_frames.saturating_add(1);
        return;
    }

    if let Some(frame_limit) = state.options.frames {
        if state.sent_frames >= frame_limit {
            state.stopped.store(true, Ordering::Relaxed);
            state.sending.store(false, Ordering::Release);
            return;
        }
    }

    let result = send_frame(
        state,
        CapturedFrame {
            pixel_format,
            plane0,
            plane0_byte_count,
            width,
            height,
            plane0_stride,
            plane1,
            plane1_byte_count,
            plane1_width,
            plane1_height,
            plane1_stride,
        },
    );
    state.sending.store(false, Ordering::Release);
    if let Err(error) = result {
        eprintln!("virtual display frame error: {error}");
        state.stopped.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug)]
struct CapturedFrame {
    pixel_format: u32,
    plane0: *const u8,
    plane0_byte_count: usize,
    width: usize,
    height: usize,
    plane0_stride: usize,
    plane1: *const u8,
    plane1_byte_count: usize,
    plane1_width: usize,
    plane1_height: usize,
    plane1_stride: usize,
}

fn send_frame(state: &mut SenderState, frame: CapturedFrame) -> Result<(), Box<dyn Error>> {
    let frame_started = Instant::now();
    let mut profile = ProfileSample::default();
    let options = &state.options;
    let expected_width = usize::from(options.width);
    let expected_height = usize::from(options.height);
    if frame.width != expected_width || frame.height != expected_height {
        return Err(format!(
            "unexpected frame size: got {}x{}, expected {}x{}",
            frame.width, frame.height, expected_width, expected_height
        )
        .into());
    }
    if frame.pixel_format == PIXEL_FORMAT_BGRA
        && (frame.plane0_stride < expected_width * 4
            || frame.plane0_byte_count < frame.plane0_stride * expected_height)
    {
        return Err("invalid BGRA frame stride or byte count".into());
    }

    let (output_width, output_height) = options.rotate.output_size(options.width, options.height);
    if let Some(path) = &options.dump_first_frame {
        if !state.first_frame_dumped {
            let rgb = copy_bgra_to_rgb(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
            );
            rgb.save(path)?;
            state.first_frame_dumped = true;
            println!("Dumped first frame to {}", path.display());
        }
    }

    let (payload_bytes, chunks, cmd_addr, fb_addr, reset_jpeg) = match options.transport {
        Transport::Jpeg => {
            ensure_bgra_capture(frame)?;
            let convert_started = Instant::now();
            let rgb = copy_bgra_to_rgb(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
            );
            profile.convert = convert_started.elapsed();
            let encode_started = Instant::now();
            let jpeg =
                turbojpeg::compress_image(&rgb, options.quality, options.subsamp.turbojpeg())?;
            profile.encode = encode_started.elapsed();
            let packet_started = Instant::now();
            let addresses = state.scheduler.next_jpeg_frame(jpeg.len());
            let flags = if addresses.reset_jpeg {
                VIDEO_FLAG_RESET_JPEG
            } else {
                0
            };
            let packet = JpegFramePacket::new_with_target_format(
                options.display_index,
                &jpeg,
                output_width,
                output_height,
                addresses.cmd_addr,
                addresses.fb_addr,
                options.jpeg_target.video_color(),
                flags,
            );
            let chunks = packet.bulk_chunks(options.max_packet_size).len();
            profile.packet = packet_started.elapsed();
            if let Some(device) = &state.device {
                let usb_started = Instant::now();
                send_chunks_with_context(
                    device,
                    &packet.bulk_chunks(options.max_packet_size),
                    "jpeg",
                )?;
                profile.usb = usb_started.elapsed();
            }
            (
                jpeg.len(),
                chunks,
                addresses.cmd_addr,
                addresses.fb_addr,
                addresses.reset_jpeg,
            )
        }
        Transport::Nv12 => {
            let nv12 = if frame.pixel_format == PIXEL_FORMAT_420F
                || frame.pixel_format == PIXEL_FORMAT_420V
            {
                copy_captured_nv12(frame, options.rotate)?
            } else {
                bgra_to_nv12(
                    frame.plane0,
                    expected_width,
                    expected_height,
                    frame.plane0_stride,
                    options.rotate,
                    options.chroma_mode,
                    options.yuv_matrix,
                    options.yuv_range,
                )?
            };
            let addresses = state.scheduler.next_jpeg_frame(nv12.len());
            let packet = RawFramePacket::nv12(
                options.display_index,
                &nv12,
                output_width,
                output_height,
                addresses.fb_addr,
                0,
            );
            let chunks = packet.bulk_chunks(options.max_packet_size).len();
            if let Some(device) = &state.device {
                send_raw_packet(
                    device,
                    &packet,
                    options.max_packet_size,
                    options.raw_bulk_mode,
                    "nv12",
                )?;
            }
            (
                packet.payload.len(),
                chunks,
                addresses.cmd_addr,
                addresses.fb_addr,
                false,
            )
        }
        Transport::Rgb24 => {
            ensure_bgra_capture(frame)?;
            let rgb = copy_bgra_to_rgb(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
            );
            let addresses = state.scheduler.next_jpeg_frame(rgb.as_raw().len());
            let packet = RawFramePacket::rgb24(
                options.display_index,
                rgb.as_raw(),
                output_width,
                output_height,
                addresses.fb_addr,
                0,
            );
            let chunks = packet.bulk_chunks(options.max_packet_size).len();
            if let Some(device) = &state.device {
                send_raw_packet(
                    device,
                    &packet,
                    options.max_packet_size,
                    options.raw_bulk_mode,
                    "rgb24",
                )?;
            }
            (
                packet.payload.len(),
                chunks,
                addresses.cmd_addr,
                addresses.fb_addr,
                false,
            )
        }
        Transport::Yv12 => {
            ensure_bgra_capture(frame)?;
            let yv12 = bgra_to_yv12(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
                options.chroma_mode,
                options.yuv_matrix,
                options.yuv_range,
            )?;
            let addresses = state.scheduler.next_jpeg_frame(yv12.len());
            let packet = RawFramePacket::yv12(
                options.display_index,
                &yv12,
                output_width,
                output_height,
                addresses.fb_addr,
                0,
            );
            let chunks = packet.bulk_chunks(options.max_packet_size).len();
            if let Some(device) = &state.device {
                send_raw_packet(
                    device,
                    &packet,
                    options.max_packet_size,
                    options.raw_bulk_mode,
                    "yv12",
                )?;
            }
            (
                packet.payload.len(),
                chunks,
                addresses.cmd_addr,
                addresses.fb_addr,
                false,
            )
        }
        Transport::Yuv444 => {
            ensure_bgra_capture(frame)?;
            let yuv444 = bgra_to_yuv444(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
                options.yuv_matrix,
                options.yuv_range,
            );
            let addresses = state.scheduler.next_jpeg_frame(yuv444.len());
            let packet = RawFramePacket::yuv444(
                options.display_index,
                &yuv444,
                output_width,
                output_height,
                addresses.fb_addr,
                0,
            );
            let chunks = packet.bulk_chunks(options.max_packet_size).len();
            if let Some(device) = &state.device {
                send_raw_packet(
                    device,
                    &packet,
                    options.max_packet_size,
                    options.raw_bulk_mode,
                    "yuv444",
                )?;
            }
            (
                packet.payload.len(),
                chunks,
                addresses.cmd_addr,
                addresses.fb_addr,
                false,
            )
        }
    };

    state.sent_frames += 1;
    state.next_send_at += state.frame_interval;
    profile.total = frame_started.elapsed();
    if options.profile {
        state.profile_stats.push(profile);
    }

    if state.sent_frames == 1 || state.sent_frames % 60 == 0 {
        println!(
            "frame={} dropped={} payload_bytes={} cmd=0x{:08x} fb=0x{:08x} reset={} chunks={}",
            state.sent_frames,
            state.dropped_frames,
            payload_bytes,
            cmd_addr,
            fb_addr,
            reset_jpeg,
            chunks
        );
        if options.profile {
            println!("{}", state.profile_stats.take().summary());
        }
    }

    if options
        .frames
        .map(|frame_limit| state.sent_frames >= frame_limit)
        .unwrap_or(false)
    {
        state.stopped.store(true, Ordering::Relaxed);
    }

    Ok(())
}

fn send_chunks_with_context(
    device: &T6Device,
    chunks: &[BulkTransferChunk<'_>],
    label: &str,
) -> Result<(), Box<dyn Error>> {
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        device
            .write_display_bulk(&chunk.header.to_bytes())
            .map_err(|error| {
                format!(
                    "{label} bulk header error at chunk {}/{} offset={} size={}: {error}",
                    chunk_index + 1,
                    chunks.len(),
                    chunk.header.packet_offset,
                    chunk.header.packet_size
                )
            })?;
        device.write_display_bulk(chunk.data).map_err(|error| {
            format!(
                "{label} bulk data error at chunk {}/{} offset={} size={}: {error}",
                chunk_index + 1,
                chunks.len(),
                chunk.header.packet_offset,
                chunk.header.packet_size
            )
        })?;
    }

    Ok(())
}

fn send_raw_packet(
    device: &T6Device,
    packet: &RawFramePacket,
    max_packet_size: u32,
    mode: RawBulkMode,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    match mode {
        RawBulkMode::Fragmented => {
            send_chunks_with_context(device, &packet.bulk_chunks(max_packet_size), label)
        }
        RawBulkMode::Single => {
            let header = BulkDmaHeader::display(
                packet.payload.len() as u32,
                packet.payload_address,
                packet.payload.len() as u32,
                0,
                false,
            );
            device
                .write_display_bulk(&header.to_bytes())
                .map_err(|error| {
                    format!(
                        "{label} single bulk header error payload_bytes={} addr=0x{:08x}: {error}",
                        packet.payload.len(),
                        packet.payload_address
                    )
                })?;
            device
                .write_display_bulk(&packet.payload)
                .map_err(|error| {
                    format!(
                        "{label} single bulk data error payload_bytes={} addr=0x{:08x}: {error}",
                        packet.payload.len(),
                        packet.payload_address
                    )
                })?;

            Ok(())
        }
    }
}

fn ensure_bgra_capture(frame: CapturedFrame) -> Result<(), Box<dyn Error>> {
    if frame.pixel_format != PIXEL_FORMAT_BGRA {
        return Err("this transport currently requires --capture-format bgra".into());
    }

    Ok(())
}

fn copy_captured_nv12(frame: CapturedFrame, rotation: Rotation) -> Result<Vec<u8>, Box<dyn Error>> {
    if rotation != Rotation::Deg0 {
        return Err("direct NV12 capture currently requires --rotate 0".into());
    }
    if frame.plane1.is_null() {
        return Err("NV12 capture frame is missing UV plane".into());
    }
    if frame.width % 2 != 0 || frame.height % 2 != 0 {
        return Err("NV12 capture requires even width and height".into());
    }

    let y_pitch = align_usize(frame.width, 16);
    let uv_pitch = y_pitch;
    if frame.plane0_stride < frame.width || frame.plane1_stride < frame.width {
        return Err("NV12 capture plane stride is smaller than frame width".into());
    }
    if frame.plane0_byte_count < frame.plane0_stride * frame.height
        || frame.plane1_byte_count < frame.plane1_stride * frame.height / 2
    {
        return Err("NV12 capture plane byte count is shorter than expected".into());
    }

    let y_size = y_pitch * frame.height;
    let uv_size = uv_pitch * frame.height / 2;
    let mut out = vec![0; y_size + uv_size];

    for y in 0..frame.height {
        let src = unsafe {
            std::slice::from_raw_parts(frame.plane0.add(y * frame.plane0_stride), frame.width)
        };
        let dst = &mut out[y * y_pitch..y * y_pitch + frame.width];
        dst.copy_from_slice(src);
    }

    let uv_height = frame.height / 2;
    let uv_copy_width = frame.width.min(frame.plane1_width * 2);
    for y in 0..uv_height.min(frame.plane1_height) {
        let src = unsafe {
            std::slice::from_raw_parts(frame.plane1.add(y * frame.plane1_stride), uv_copy_width)
        };
        let dst_offset = y_size + y * uv_pitch;
        out[dst_offset..dst_offset + uv_copy_width].copy_from_slice(src);
    }

    Ok(out)
}

fn bgra_to_yuv444(
    bgra: *const u8,
    width: usize,
    height: usize,
    stride: usize,
    rotation: Rotation,
    matrix: YuvMatrix,
    range: YuvRange,
) -> Vec<u8> {
    let (output_width, output_height) = rotated_size(width, height, rotation);
    let mut out = Vec::with_capacity(output_width * output_height * 3);

    for out_y in 0..output_height {
        for out_x in 0..output_width {
            let (src_x, src_y) = source_coordinate(out_x, out_y, width, height, rotation);
            let pixel =
                unsafe { std::slice::from_raw_parts(bgra.add(src_y * stride + src_x * 4), 4) };
            let (y, u, v) = bgr_to_yuv(pixel[2], pixel[1], pixel[0], matrix, range);
            out.push(y);
            out.push(u);
            out.push(v);
        }
    }

    out
}

fn bgra_to_nv12(
    bgra: *const u8,
    width: usize,
    height: usize,
    stride: usize,
    rotation: Rotation,
    chroma_mode: ChromaMode,
    matrix: YuvMatrix,
    range: YuvRange,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let (output_width, output_height) = rotated_size(width, height, rotation);
    if output_width % 2 != 0 || output_height % 2 != 0 {
        return Err("NV12 requires even output width and height".into());
    }

    let y_pitch = align_usize(output_width, 16);
    let uv_pitch = y_pitch;
    let y_size = y_pitch * output_height;
    let uv_size = uv_pitch * output_height / 2;
    let mut out = vec![0; y_size + uv_size];

    for out_y in (0..output_height).step_by(2) {
        for out_x in (0..output_width).step_by(2) {
            let mut chroma = ChromaAccumulator::new(chroma_mode);

            for dy in 0..2 {
                for dx in 0..2 {
                    let (src_x, src_y) =
                        source_coordinate(out_x + dx, out_y + dy, width, height, rotation);
                    let pixel = unsafe {
                        std::slice::from_raw_parts(bgra.add(src_y * stride + src_x * 4), 4)
                    };
                    let (y, u, v) = bgr_to_yuv(pixel[2], pixel[1], pixel[0], matrix, range);
                    out[(out_y + dy) * y_pitch + out_x + dx] = y;
                    chroma.push(u, v);
                }
            }

            let (u, v) = chroma.finish();
            let uv_offset = y_size + (out_y / 2) * uv_pitch + out_x;
            out[uv_offset] = u;
            out[uv_offset + 1] = v;
        }
    }

    Ok(out)
}

fn bgra_to_yv12(
    bgra: *const u8,
    width: usize,
    height: usize,
    stride: usize,
    rotation: Rotation,
    chroma_mode: ChromaMode,
    matrix: YuvMatrix,
    range: YuvRange,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let (output_width, output_height) = rotated_size(width, height, rotation);
    if output_width % 2 != 0 || output_height % 2 != 0 {
        return Err("YV12 requires even output width and height".into());
    }

    let y_pitch = align_usize(output_width, 16);
    let uv_pitch = align_usize(output_width / 2, 16);
    let y_size = y_pitch * output_height;
    let uv_size = uv_pitch * output_height / 2;
    let u_start = y_size;
    let v_start = y_size + uv_size;
    let mut out = vec![0; y_size + uv_size * 2];

    for out_y in (0..output_height).step_by(2) {
        for out_x in (0..output_width).step_by(2) {
            let mut chroma = ChromaAccumulator::new(chroma_mode);

            for dy in 0..2 {
                for dx in 0..2 {
                    let (src_x, src_y) =
                        source_coordinate(out_x + dx, out_y + dy, width, height, rotation);
                    let pixel = unsafe {
                        std::slice::from_raw_parts(bgra.add(src_y * stride + src_x * 4), 4)
                    };
                    let (y, u, v) = bgr_to_yuv(pixel[2], pixel[1], pixel[0], matrix, range);
                    out[(out_y + dy) * y_pitch + out_x + dx] = y;
                    chroma.push(u, v);
                }
            }

            let (u, v) = chroma.finish();
            let uv_offset = (out_y / 2) * uv_pitch + out_x / 2;
            out[u_start + uv_offset] = u;
            out[v_start + uv_offset] = v;
        }
    }

    Ok(out)
}

#[derive(Clone, Copy, Debug)]
struct ChromaAccumulator {
    mode: ChromaMode,
    count: u16,
    u_sum: u16,
    v_sum: u16,
    selected_u: u8,
    selected_v: u8,
    selected_score: i32,
}

impl ChromaAccumulator {
    fn new(mode: ChromaMode) -> Self {
        Self {
            mode,
            count: 0,
            u_sum: 0,
            v_sum: 0,
            selected_u: 128,
            selected_v: 128,
            selected_score: -1,
        }
    }

    fn push(&mut self, u: u8, v: u8) {
        self.count += 1;
        self.u_sum += u16::from(u);
        self.v_sum += u16::from(v);

        match self.mode {
            ChromaMode::Average => {}
            ChromaMode::TopLeft => {
                if self.count == 1 {
                    self.selected_u = u;
                    self.selected_v = v;
                }
            }
            ChromaMode::Saturated => {
                let du = i32::from(u) - 128;
                let dv = i32::from(v) - 128;
                let score = du * du + dv * dv;
                if score > self.selected_score {
                    self.selected_score = score;
                    self.selected_u = u;
                    self.selected_v = v;
                }
            }
        }
    }

    fn finish(self) -> (u8, u8) {
        match self.mode {
            ChromaMode::Average => (
                (self.u_sum / self.count) as u8,
                (self.v_sum / self.count) as u8,
            ),
            ChromaMode::Saturated | ChromaMode::TopLeft => (self.selected_u, self.selected_v),
        }
    }
}

fn rotated_size(width: usize, height: usize, rotation: Rotation) -> (usize, usize) {
    match rotation {
        Rotation::Deg0 | Rotation::Deg180 => (width, height),
        Rotation::Deg90 | Rotation::Deg270 => (height, width),
    }
}

#[cfg(test)]
fn rgb_to_nv12(rgb: &[u8], width: u16, height: u16) -> Result<Vec<u8>, Box<dyn Error>> {
    let width = usize::from(width);
    let height = usize::from(height);
    if width % 2 != 0 || height % 2 != 0 {
        return Err("NV12 requires even width and height".into());
    }

    let y_pitch = align_usize(width, 16);
    let uv_pitch = y_pitch;
    let y_size = y_pitch * height;
    let uv_size = uv_pitch * height / 2;
    let mut out = vec![0; y_size + uv_size];

    fill_y_plane(rgb, width, height, y_pitch, &mut out[..y_size]);
    for by in (0..height).step_by(2) {
        for bx in (0..width).step_by(2) {
            let (u, v) = average_uv_2x2(rgb, width, bx, by);
            let offset = y_size + (by / 2) * uv_pitch + bx;
            out[offset] = u;
            out[offset + 1] = v;
        }
    }

    Ok(out)
}

#[cfg(test)]
fn rgb_to_yv12(rgb: &[u8], width: u16, height: u16) -> Result<Vec<u8>, Box<dyn Error>> {
    let width = usize::from(width);
    let height = usize::from(height);
    if width % 2 != 0 || height % 2 != 0 {
        return Err("YV12 requires even width and height".into());
    }

    let y_pitch = align_usize(width, 16);
    let uv_pitch = align_usize(width / 2, 16);
    let y_size = y_pitch * height;
    let uv_size = uv_pitch * height / 2;
    let u_start = y_size;
    let v_start = y_size + uv_size;
    let mut out = vec![0; y_size + uv_size * 2];

    fill_y_plane(rgb, width, height, y_pitch, &mut out[..y_size]);
    for by in (0..height).step_by(2) {
        for bx in (0..width).step_by(2) {
            let (u, v) = average_uv_2x2(rgb, width, bx, by);
            let uv_offset = (by / 2) * uv_pitch + bx / 2;
            out[u_start + uv_offset] = u;
            out[v_start + uv_offset] = v;
        }
    }

    Ok(out)
}

#[cfg(test)]
fn fill_y_plane(rgb: &[u8], width: usize, height: usize, y_pitch: usize, y_plane: &mut [u8]) {
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 3;
            y_plane[y * y_pitch + x] = rgb_to_y(rgb[offset], rgb[offset + 1], rgb[offset + 2]);
        }
    }
}

#[cfg(test)]
fn average_uv_2x2(rgb: &[u8], width: usize, x: usize, y: usize) -> (u8, u8) {
    let mut u_sum = 0u16;
    let mut v_sum = 0u16;

    for dy in 0..2 {
        for dx in 0..2 {
            let offset = ((y + dy) * width + x + dx) * 3;
            let (_y, u, v) = rgb_to_yuv(rgb[offset], rgb[offset + 1], rgb[offset + 2]);
            u_sum += u16::from(u);
            v_sum += u16::from(v);
        }
    }

    ((u_sum / 4) as u8, (v_sum / 4) as u8)
}

#[cfg(test)]
fn rgb_to_y(r: u8, g: u8, b: u8) -> u8 {
    rgb_to_yuv(r, g, b).0
}

#[cfg(test)]
fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    bgr_to_yuv(r, g, b, YuvMatrix::Bt601, YuvRange::Full)
}

fn bgr_to_yuv(r: u8, g: u8, b: u8, matrix: YuvMatrix, range: YuvRange) -> (u8, u8, u8) {
    let r = i32::from(r);
    let g = i32::from(g);
    let b = i32::from(b);
    let (y, u, v) = match (matrix, range) {
        (YuvMatrix::Bt601, YuvRange::Full) => {
            let y = (77 * r + 150 * g + 29 * b + 128) >> 8;
            let u = ((-43 * r - 85 * g + 128 * b + 128) >> 8) + 128;
            let v = ((128 * r - 107 * g - 21 * b + 128) >> 8) + 128;
            (y, u, v)
        }
        (YuvMatrix::Bt709, YuvRange::Full) => {
            let y = (54 * r + 183 * g + 18 * b + 128) >> 8;
            let u = ((-29 * r - 99 * g + 128 * b + 128) >> 8) + 128;
            let v = ((128 * r - 116 * g - 12 * b + 128) >> 8) + 128;
            (y, u, v)
        }
        (YuvMatrix::Bt601, YuvRange::Video) => {
            let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
            (y, u, v)
        }
        (YuvMatrix::Bt709, YuvRange::Video) => {
            let y = ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16;
            let u = ((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
            (y, u, v)
        }
    };

    (clamp_u8(y), clamp_u8(u), clamp_u8(v))
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn align_usize(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn copy_bgra_to_rgb(
    bgra: *const u8,
    width: usize,
    height: usize,
    stride: usize,
    rotation: Rotation,
) -> RgbImage {
    let (output_width, output_height) = match rotation {
        Rotation::Deg0 | Rotation::Deg180 => (width, height),
        Rotation::Deg90 | Rotation::Deg270 => (height, width),
    };
    let mut rgb = Vec::with_capacity(width * height * 3);

    for out_y in 0..output_height {
        for out_x in 0..output_width {
            let (src_x, src_y) = source_coordinate(out_x, out_y, width, height, rotation);
            let offset = src_y * stride + src_x * 4;
            let pixel = unsafe { std::slice::from_raw_parts(bgra.add(offset), 4) };
            rgb.push(pixel[2]);
            rgb.push(pixel[1]);
            rgb.push(pixel[0]);
        }
    }

    RgbImage::from_raw(output_width as u32, output_height as u32, rgb)
        .expect("RGB buffer has expected size")
}

fn source_coordinate(
    out_x: usize,
    out_y: usize,
    width: usize,
    height: usize,
    rotation: Rotation,
) -> (usize, usize) {
    match rotation {
        Rotation::Deg0 => (out_x, out_y),
        Rotation::Deg90 => (out_y, height - 1 - out_x),
        Rotation::Deg180 => (width - 1 - out_x, height - 1 - out_y),
        Rotation::Deg270 => (width - 1 - out_y, out_x),
    }
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut display_index = 1;
    let mut width = 1920;
    let mut height = 1080;
    let mut rotate = Rotation::Deg0;
    let mut fps = 60;
    let mut frames = None;
    let mut quality = 95;
    let mut subsamp = JpegSubsampling::Yuv420;
    let mut jpeg_target = JpegTarget::Nv12;
    let mut chroma_mode = ChromaMode::Average;
    let mut yuv_matrix = YuvMatrix::Bt601;
    let mut yuv_range = YuvRange::Full;
    let mut transport = Transport::Jpeg;
    let mut capture_format = CaptureFormat::Bgra;
    let mut raw_bulk_mode = RawBulkMode::Fragmented;
    let mut ready = false;
    let mut power_on = false;
    let mut profile = false;
    let mut dry_run = false;
    let mut ram_size_mb = None;
    let mut usb_timeout_ms = 3000;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut dump_first_frame = None;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--display" => display_index = next_value(&mut args, "--display")?.parse()?,
            "--width" => width = next_value(&mut args, "--width")?.parse()?,
            "--height" => height = next_value(&mut args, "--height")?.parse()?,
            "--rotate" => {
                rotate = parse_rotation(&next_value(&mut args, "--rotate")?)?;
            }
            "--fps" => fps = next_value(&mut args, "--fps")?.parse()?,
            "--frames" => frames = Some(next_value(&mut args, "--frames")?.parse()?),
            "--quality" => quality = next_value(&mut args, "--quality")?.parse()?,
            "--subsamp" => subsamp = parse_subsampling(&next_value(&mut args, "--subsamp")?)?,
            "--jpeg-target" => {
                jpeg_target = parse_jpeg_target(&next_value(&mut args, "--jpeg-target")?)?
            }
            "--chroma-mode" => {
                chroma_mode = parse_chroma_mode(&next_value(&mut args, "--chroma-mode")?)?
            }
            "--yuv-matrix" => {
                yuv_matrix = parse_yuv_matrix(&next_value(&mut args, "--yuv-matrix")?)?
            }
            "--yuv-range" => yuv_range = parse_yuv_range(&next_value(&mut args, "--yuv-range")?)?,
            "--capture-format" => {
                capture_format = parse_capture_format(&next_value(&mut args, "--capture-format")?)?
            }
            "--raw-bulk" => {
                raw_bulk_mode = parse_raw_bulk_mode(&next_value(&mut args, "--raw-bulk")?)?
            }
            "--transport" => transport = parse_transport(&next_value(&mut args, "--transport")?)?,
            "--ready" => ready = true,
            "--power-on" => power_on = true,
            "--profile" => profile = true,
            "--dry-run" => dry_run = true,
            "--ram-size-mb" => ram_size_mb = Some(next_value(&mut args, "--ram-size-mb")?.parse()?),
            "--usb-timeout-ms" => {
                usb_timeout_ms = next_value(&mut args, "--usb-timeout-ms")?.parse()?
            }
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "--dump-first-frame" => {
                dump_first_frame = Some(PathBuf::from(next_value(&mut args, "--dump-first-frame")?))
            }
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
        display_index,
        width,
        height,
        rotate,
        fps,
        frames,
        quality,
        subsamp,
        jpeg_target,
        chroma_mode,
        yuv_matrix,
        yuv_range,
        transport,
        capture_format,
        raw_bulk_mode,
        ready,
        power_on,
        profile,
        dry_run,
        ram_size_mb,
        usb_timeout_ms,
        max_packet_size,
        dump_first_frame,
    })
}

fn parse_jpeg_target(value: &str) -> Result<JpegTarget, Box<dyn Error>> {
    match value {
        "nv12" => Ok(JpegTarget::Nv12),
        "yv12" => Ok(JpegTarget::Yv12),
        "yuv444" => Ok(JpegTarget::Yuv444),
        _ => Err("--jpeg-target must be one of nv12, yv12, yuv444".into()),
    }
}

fn parse_yuv_matrix(value: &str) -> Result<YuvMatrix, Box<dyn Error>> {
    match value {
        "bt601" | "601" => Ok(YuvMatrix::Bt601),
        "bt709" | "709" => Ok(YuvMatrix::Bt709),
        _ => Err("--yuv-matrix must be one of bt601, bt709".into()),
    }
}

fn parse_yuv_range(value: &str) -> Result<YuvRange, Box<dyn Error>> {
    match value {
        "full" => Ok(YuvRange::Full),
        "video" | "limited" => Ok(YuvRange::Video),
        _ => Err("--yuv-range must be one of full, video".into()),
    }
}

fn parse_raw_bulk_mode(value: &str) -> Result<RawBulkMode, Box<dyn Error>> {
    match value {
        "fragmented" => Ok(RawBulkMode::Fragmented),
        "single" => Ok(RawBulkMode::Single),
        _ => Err("--raw-bulk must be one of fragmented, single".into()),
    }
}

fn parse_capture_format(value: &str) -> Result<CaptureFormat, Box<dyn Error>> {
    match value {
        "bgra" => Ok(CaptureFormat::Bgra),
        "420f" => Ok(CaptureFormat::Nv12FullRange),
        "420v" => Ok(CaptureFormat::Nv12VideoRange),
        _ => Err("--capture-format must be one of bgra, 420f, 420v".into()),
    }
}

fn parse_chroma_mode(value: &str) -> Result<ChromaMode, Box<dyn Error>> {
    match value {
        "average" => Ok(ChromaMode::Average),
        "saturated" => Ok(ChromaMode::Saturated),
        "top-left" => Ok(ChromaMode::TopLeft),
        _ => Err("--chroma-mode must be one of average, saturated, top-left".into()),
    }
}

fn parse_transport(value: &str) -> Result<Transport, Box<dyn Error>> {
    match value {
        "jpeg" => Ok(Transport::Jpeg),
        "nv12" => Ok(Transport::Nv12),
        "rgb24" => Ok(Transport::Rgb24),
        "yv12" => Ok(Transport::Yv12),
        "yuv444" => Ok(Transport::Yuv444),
        _ => Err("--transport must be one of jpeg, nv12, rgb24, yv12, yuv444".into()),
    }
}

fn parse_subsampling(value: &str) -> Result<JpegSubsampling, Box<dyn Error>> {
    match value {
        "420" | "4:2:0" => Ok(JpegSubsampling::Yuv420),
        "422" | "4:2:2" => Ok(JpegSubsampling::Yuv422),
        "444" | "4:4:4" => Ok(JpegSubsampling::Yuv444),
        _ => Err("--subsamp must be one of 420, 422, 444".into()),
    }
}

fn parse_rotation(value: &str) -> Result<Rotation, Box<dyn Error>> {
    match value {
        "0" => Ok(Rotation::Deg0),
        "90" => Ok(Rotation::Deg90),
        "180" => Ok(Rotation::Deg180),
        "270" => Ok(Rotation::Deg270),
        _ => Err("--rotate must be one of 0, 90, 180, 270".into()),
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

fn print_help() {
    println!(
        "Usage: t6-virtual-display [options]\n\
\n\
Options:\n\
    --display 0|1           T6 display index, default 1\n\
    --width N               Virtual display width, default 1920\n\
    --height N              Virtual display height, default 1080\n\
    --rotate 0|90|180|270   Rotate output before sending, default 0\n\
                            For portrait-on-landscape output, use --width 1080 --height 1920 --rotate 90\n\
    --fps N                 Virtual display refresh/send cap, default 60\n\
    --frames N              Stop after N sent frames\n\
    --quality N             TurboJPEG quality 1..100, default 95\n\
    --subsamp 420|422|444   JPEG chroma subsampling, default 420\n\
    --jpeg-target nv12|yv12|yuv444\n\
                            Target format for JPEG decoder output, default nv12\n\
    --chroma-mode average|saturated|top-left\n\
                            Chroma selection for nv12/yv12, default average\n\
    --yuv-matrix bt601|bt709\n\
                            RGB to YUV matrix for BGRA capture, default bt601\n\
    --yuv-range full|video  RGB to YUV range for BGRA capture, default full\n\
    --capture-format bgra|420f|420v\n\
                            CGDisplayStream capture pixel format, default bgra\n\
    --raw-bulk fragmented|single\n\
                            Raw nv12/yv12/rgb24 USB transfer mode, default fragmented\n\
    --transport jpeg|nv12|rgb24|yv12|yuv444\n\
                            Frame transport, default jpeg\n\
    --ready                 Send software-ready before capture\n\
    --power-on              Send monitor power-on before capture\n\
    --profile               Print average convert/encode/packet/USB timings every 60 sent frames\n\
    --dry-run               Capture/encode/packetize but do not open USB or send\n\
    --ram-size-mb N         RAM size for dry-run address planning, default 58\n\
    --usb-timeout-ms N      USB transfer timeout, default 3000\n\
    --max-packet N          Bulk fragment size, default 0x19000\n\
    --dump-first-frame PATH Save the first captured BGRA frame after RGB conversion"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        ChromaMode, Rotation, YuvMatrix, YuvRange, bgra_to_nv12, bgra_to_yv12, rgb_to_nv12,
        rgb_to_yv12, source_coordinate,
    };

    #[test]
    fn rotation_output_size_swaps_dimensions_for_quarter_turns() {
        assert_eq!(Rotation::Deg0.output_size(1920, 1080), (1920, 1080));
        assert_eq!(Rotation::Deg90.output_size(1080, 1920), (1920, 1080));
        assert_eq!(Rotation::Deg180.output_size(1920, 1080), (1920, 1080));
        assert_eq!(Rotation::Deg270.output_size(1080, 1920), (1920, 1080));
    }

    #[test]
    fn rotation_90_maps_output_to_source_clockwise() {
        assert_eq!(source_coordinate(0, 0, 3, 2, Rotation::Deg90), (0, 1));
        assert_eq!(source_coordinate(1, 0, 3, 2, Rotation::Deg90), (0, 0));
        assert_eq!(source_coordinate(0, 2, 3, 2, Rotation::Deg90), (2, 1));
    }

    #[test]
    fn rotation_270_maps_output_to_source_counter_clockwise() {
        assert_eq!(source_coordinate(0, 0, 3, 2, Rotation::Deg270), (2, 0));
        assert_eq!(source_coordinate(1, 0, 3, 2, Rotation::Deg270), (2, 1));
        assert_eq!(source_coordinate(0, 2, 3, 2, Rotation::Deg270), (0, 0));
    }

    #[test]
    fn nv12_uses_aligned_y_and_interleaved_uv_planes() {
        let rgb = vec![0; 2 * 2 * 3];
        let nv12 = rgb_to_nv12(&rgb, 2, 2).unwrap();

        assert_eq!(nv12.len(), 16 * 2 + 16);
    }

    #[test]
    fn yv12_uses_aligned_planar_uv_planes() {
        let rgb = vec![0; 2 * 2 * 3];
        let yv12 = rgb_to_yv12(&rgb, 2, 2).unwrap();

        assert_eq!(yv12.len(), 16 * 2 + 16 + 16);
    }

    #[test]
    fn direct_bgra_to_nv12_matches_rgb_path_without_rotation() {
        let rgb = sample_rgb_4x2();
        let bgra = bgra_from_rgb(&rgb);
        let direct = bgra_to_nv12(
            bgra.as_ptr(),
            4,
            2,
            4 * 4,
            Rotation::Deg0,
            ChromaMode::Average,
            YuvMatrix::Bt601,
            YuvRange::Full,
        )
        .unwrap();
        let via_rgb = rgb_to_nv12(&rgb, 4, 2).unwrap();

        assert_eq!(direct, via_rgb);
    }

    #[test]
    fn direct_bgra_to_yv12_matches_rgb_path_without_rotation() {
        let rgb = sample_rgb_4x2();
        let bgra = bgra_from_rgb(&rgb);
        let direct = bgra_to_yv12(
            bgra.as_ptr(),
            4,
            2,
            4 * 4,
            Rotation::Deg0,
            ChromaMode::Average,
            YuvMatrix::Bt601,
            YuvRange::Full,
        )
        .unwrap();
        let via_rgb = rgb_to_yv12(&rgb, 4, 2).unwrap();

        assert_eq!(direct, via_rgb);
    }

    fn sample_rgb_4x2() -> Vec<u8> {
        vec![
            255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 32, 64, 96, 96, 64, 32, 12, 24, 36,
            200, 180, 160,
        ]
    }

    fn bgra_from_rgb(rgb: &[u8]) -> Vec<u8> {
        let mut bgra = Vec::with_capacity(rgb.len() / 3 * 4);
        for pixel in rgb.chunks_exact(3) {
            bgra.push(pixel[2]);
            bgra.push(pixel[1]);
            bgra.push(pixel[0]);
            bgra.push(255);
        }

        bgra
    }
}
