use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::ffi::CStr;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use image::RgbImage;
use t6proto::usb::T6Device;
use t6proto::{
    BulkDmaHeader, BulkTransferChunk, DEFAULT_MAX_BULK_PACKET_SIZE, FrameScheduler,
    JpegFramePacket, RawFramePacket, Type4Mode6SetupPacket, Type7JpegTilePacket, VIDEO_COLOR_NV12,
    VIDEO_COLOR_YUV444, VIDEO_COLOR_YV12, VIDEO_FLAG_RESET_JPEG, VramLayout,
    type7_mode6_cmd_offset,
};
use turbojpeg::Subsamp;
use turbojpeg_sys as tj;

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
    usize,
    usize,
    usize,
    usize,
    usize,
    u64,
    *const DirtyRect,
    usize,
    *mut c_void,
);

const TYPE7_MODE6_ALLOCATED_BASES: [u32; 3] = [0x0250_0400, 0x0295_56c0, 0x02da_a980];
const TYPE7_SETUP_SLOT_ORDER: [usize; 3] = [2, 0, 1];

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

#[repr(C)]
struct VImageBuffer {
    data: *mut c_void,
    height: usize,
    width: usize,
    row_bytes: usize,
}

unsafe extern "C" {
    fn vImageRotate90_ARGB8888(
        src: *const VImageBuffer,
        dest: *const VImageBuffer,
        rotation_constant: u8,
        back_color: *const u8,
        flags: u32,
    ) -> isize;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn t6_vt_jpeg_encoder_create(width: usize, height: usize, quality: f32) -> *mut c_void;
    fn t6_vt_jpeg_encoder_destroy(encoder: *mut c_void);
    fn t6_vt_jpeg_encoder_encode_bgra(
        encoder: *mut c_void,
        bgra: *const u8,
        width: usize,
        height: usize,
        stride: usize,
        quality: f32,
        jpeg_data: *mut *const u8,
        jpeg_len: *mut usize,
    ) -> i32;
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
    adaptive_quality: bool,
    min_quality: i32,
    subsamp: JpegSubsampling,
    jpeg_encoder: JpegEncoder,
    jpeg_target: JpegTarget,
    chroma_mode: ChromaMode,
    yuv_matrix: YuvMatrix,
    yuv_range: YuvRange,
    transport: Transport,
    capture_format: CaptureFormat,
    raw_bulk_mode: RawBulkMode,
    ready: bool,
    power_on: bool,
    reset_jpeg_engine: bool,
    profile: bool,
    report_every: u32,
    async_send: bool,
    frame_throttle: bool,
    drop_late_frames: bool,
    unsafe_generated_type7: bool,
    type7_rect_mode: Type7RectMode,
    type7_ring_mode: Type7RingMode,
    type7_refresh_frames: u32,
    type7_min_band_ratio: f64,
    type7_min_local_area_ratio: f64,
    type7_max_tile_ratio: f64,
    type7_large_tile_ratio: f64,
    type7_large_tile_quality: Option<i32>,
    type7_large_tile_fps_divisor: u32,
    type7_setup_mode: Type7SetupMode,
    type7_jpeg_component_ids: Type7JpegComponentIds,
    type7_wait_setup_ack_ms: u64,
    type7_setup_only: bool,
    dirty_mode: DirtyMode,
    dry_run: bool,
    ram_size_mb: Option<u8>,
    usb_timeout_ms: u64,
    wait_interrupt_ms: u64,
    dump_interrupts: u32,
    max_packet_size: u32,
    dump_first_frame: Option<PathBuf>,
    dump_first_frame_delay_ms: u64,
    dump_type7_packets: Option<PathBuf>,
    type7_cmd_addr: Option<u32>,
    type7_cmd_wrap_addr: Option<u32>,
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
enum JpegEncoder {
    TurboBgra,
    T6Yuv420,
    VideoToolbox,
    VideoToolboxFullTurboTiles,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transport {
    Jpeg,
    JpegType7,
    Nv12,
    Nv12Type7,
    Rgb24,
    Yv12,
    Yuv444,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Type7RectMode {
    Auto,
    Local,
    HorizontalBand,
    VerticalBand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Type7RingMode {
    Windows,
    Legacy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Type7SetupMode {
    LegacyFullJpeg,
    SyntheticType4,
    PairedType4Type7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Type7JpegComponentIds {
    ZeroBased,
    Original,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirtyMode {
    Off,
    Log,
    Bbox,
    TileSend,
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
    jpeg_compressor: Option<FastJpegCompressor>,
    vt_jpeg_compressor: Option<VtJpegCompressor>,
    vt_jpeg_tile_compressors: HashMap<(usize, usize), VtJpegCompressor>,
    rgb_scratch: Vec<u8>,
    bgra_scratch: Vec<u8>,
    dirty_bgra_scratch: Vec<u8>,
    y_plane_scratch: Vec<u8>,
    u_plane_scratch: Vec<u8>,
    v_plane_scratch: Vec<u8>,
    current_quality: i32,
    remaining_interrupt_dumps: u32,
    next_fence_id: u32,
    frame_interval: Duration,
    next_send_at: Instant,
    started_at: Instant,
    last_report_at: Instant,
    last_report_sent_frames: u32,
    sent_frames: u32,
    dropped_frames: u32,
    throttled_frames: u32,
    busy_frames: u32,
    late_frames: u32,
    current_display_fb_addr: Option<u32>,
    current_type7_slot: usize,
    current_type7_allocated_base: u32,
    last_type7_dirty_bbox: Option<(usize, usize, usize, usize)>,
    dumped_type7_packets: u32,
    profile_stats: ProfileStats,
    first_frame_dumped: bool,
    sending: AtomicBool,
    stopped: AtomicBool,
}

unsafe impl Send for SenderState {}

struct FastJpegCompressor {
    handle: tj::tjhandle,
    output: *mut u8,
    output_len: usize,
    output_capacity: usize,
    configured_quality: i32,
    subsamp: Subsamp,
}

unsafe impl Send for FastJpegCompressor {}

struct VtJpegCompressor {
    encoder: *mut c_void,
    width: usize,
    height: usize,
}

unsafe impl Send for VtJpegCompressor {}

impl VtJpegCompressor {
    #[cfg(target_os = "macos")]
    fn new(width: usize, height: usize, quality: i32) -> Result<Self, Box<dyn Error>> {
        let encoder = unsafe { t6_vt_jpeg_encoder_create(width, height, quality_to_vt(quality)) };
        if encoder.is_null() {
            return Err("t6_vt_jpeg_encoder_create failed".into());
        }
        Ok(Self {
            encoder,
            width,
            height,
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn new(_width: usize, _height: usize, _quality: i32) -> Result<Self, Box<dyn Error>> {
        Err("VideoToolbox JPEG encoder is only supported on macOS".into())
    }

    fn compress_bgra_ptr(
        &mut self,
        bgra: *const u8,
        width: usize,
        pitch: usize,
        height: usize,
        quality: i32,
    ) -> Result<&[u8], Box<dyn Error>> {
        if width != self.width || height != self.height {
            return Err("VideoToolbox JPEG encoder dimensions changed".into());
        }
        #[cfg(target_os = "macos")]
        {
            let mut jpeg_data: *const u8 = ptr::null();
            let mut jpeg_len = 0usize;
            let status = unsafe {
                t6_vt_jpeg_encoder_encode_bgra(
                    self.encoder,
                    bgra,
                    width,
                    height,
                    pitch,
                    quality_to_vt(quality),
                    &mut jpeg_data,
                    &mut jpeg_len,
                )
            };
            if status != 0 {
                return Err(format!("VideoToolbox JPEG encode failed: {status}").into());
            }
            if jpeg_data.is_null() || jpeg_len == 0 {
                return Err("VideoToolbox JPEG encode returned empty output".into());
            }
            Ok(unsafe { std::slice::from_raw_parts(jpeg_data, jpeg_len) })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (bgra, pitch, quality);
            Err("VideoToolbox JPEG encoder is only supported on macOS".into())
        }
    }
}

impl Drop for VtJpegCompressor {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            t6_vt_jpeg_encoder_destroy(self.encoder);
        }
    }
}

fn quality_to_vt(quality: i32) -> f32 {
    (quality.clamp(1, 100) as f32) / 100.0
}

impl FastJpegCompressor {
    fn new(quality: i32, subsamp: Subsamp) -> Result<Self, Box<dyn Error>> {
        let handle = unsafe { tj::tj3Init(tj::TJINIT_TJINIT_COMPRESS as i32) };
        if handle.is_null() {
            return Err("tj3Init failed".into());
        }
        let mut compressor = Self {
            handle,
            output: ptr::null_mut(),
            output_len: 0,
            output_capacity: 0,
            configured_quality: quality,
            subsamp,
        };
        compressor.set_param(tj::TJPARAM_TJPARAM_QUALITY, quality)?;
        compressor.set_param(tj::TJPARAM_TJPARAM_SUBSAMP, subsamp as i32)?;
        compressor.set_param(tj::TJPARAM_TJPARAM_OPTIMIZE, 0)?;
        compressor.set_param(tj::TJPARAM_TJPARAM_FASTDCT, 1)?;
        compressor.set_param(tj::TJPARAM_TJPARAM_NOREALLOC, 1)?;
        Ok(compressor)
    }

    fn compress_bgra_ptr(
        &mut self,
        bgra: *const u8,
        width: usize,
        pitch: usize,
        height: usize,
        quality: i32,
    ) -> Result<&[u8], Box<dyn Error>> {
        self.compress_pixels(
            bgra,
            width,
            pitch,
            height,
            tj::TJPF_TJPF_BGRA as i32,
            quality,
        )
    }

    fn compress_t6_yuv420_bgra_ptr(
        &mut self,
        bgra: *const u8,
        width: usize,
        pitch: usize,
        height: usize,
        quality: i32,
        y_plane: &mut Vec<u8>,
        u_plane: &mut Vec<u8>,
        v_plane: &mut Vec<u8>,
    ) -> Result<&[u8], Box<dyn Error>> {
        if width % 2 != 0 || height % 2 != 0 {
            return Err("t6-yuv420 JPEG encoder requires even width and height".into());
        }
        if self.subsamp != Subsamp::Sub2x2 {
            return Err("t6-yuv420 JPEG encoder requires --subsamp 420".into());
        }
        if self.configured_quality != quality {
            self.set_param(tj::TJPARAM_TJPARAM_QUALITY, quality)?;
            self.configured_quality = quality;
        }
        self.ensure_output_capacity(width, height)?;
        fill_t6_yuv420_from_bgra(bgra, width, pitch, height, y_plane, u_plane, v_plane);

        let chroma_width = width / 2;
        let planes = [y_plane.as_ptr(), u_plane.as_ptr(), v_plane.as_ptr()];
        let strides = [
            width.try_into()?,
            chroma_width.try_into()?,
            chroma_width.try_into()?,
        ];
        let mut output_len = self.output_capacity as u64;
        let result = unsafe {
            tj::tj3CompressFromYUVPlanes8(
                self.handle,
                planes.as_ptr(),
                width.try_into()?,
                strides.as_ptr(),
                height.try_into()?,
                &mut self.output,
                &mut output_len,
            )
        };
        self.output_len = output_len.try_into()?;
        if result != 0 {
            return Err(self.error().into());
        }
        if self.output.is_null() {
            return Err("tj3CompressFromYUVPlanes8 returned null output".into());
        }

        Ok(unsafe { std::slice::from_raw_parts(self.output, self.output_len) })
    }

    fn compress_pixels(
        &mut self,
        pixels: *const u8,
        width: usize,
        pitch: usize,
        height: usize,
        pixel_format: i32,
        quality: i32,
    ) -> Result<&[u8], Box<dyn Error>> {
        if self.configured_quality != quality {
            self.set_param(tj::TJPARAM_TJPARAM_QUALITY, quality)?;
            self.configured_quality = quality;
        }
        self.ensure_output_capacity(width, height)?;

        let mut output_len = self.output_capacity as u64;
        let result = unsafe {
            tj::tj3Compress8(
                self.handle,
                pixels,
                width.try_into()?,
                pitch.try_into()?,
                height.try_into()?,
                pixel_format,
                &mut self.output,
                &mut output_len,
            )
        };
        self.output_len = output_len.try_into()?;
        if result != 0 {
            return Err(self.error().into());
        }
        if self.output.is_null() {
            return Err("tj3Compress8 returned null output".into());
        }

        Ok(unsafe { std::slice::from_raw_parts(self.output, self.output_len) })
    }

    fn set_param(&mut self, param: tj::TJPARAM, value: i32) -> Result<(), Box<dyn Error>> {
        let result = unsafe { tj::tj3Set(self.handle, param as i32, value) };
        if result != 0 {
            return Err(self.error().into());
        }
        Ok(())
    }

    fn ensure_output_capacity(
        &mut self,
        width: usize,
        height: usize,
    ) -> Result<(), Box<dyn Error>> {
        let capacity_raw = unsafe {
            tj::tj3JPEGBufSize(width.try_into()?, height.try_into()?, self.subsamp as i32)
        };
        if capacity_raw == 0 {
            return Err(self.error().into());
        }
        let capacity: usize = capacity_raw.try_into()?;
        if !self.output.is_null() && self.output_capacity >= capacity {
            return Ok(());
        }

        if !self.output.is_null() {
            unsafe { tj::tj3Free(self.output.cast()) };
            self.output = ptr::null_mut();
            self.output_capacity = 0;
        }
        let output = unsafe { tj::tj3Alloc(capacity_raw) };
        if output.is_null() {
            return Err("tj3Alloc failed".into());
        }
        self.output = output.cast();
        self.output_capacity = capacity;
        Ok(())
    }

    fn error(&self) -> String {
        let error = unsafe { tj::tj3GetErrorStr(self.handle) };
        if error.is_null() {
            return "TurboJPEG error".to_string();
        }
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for FastJpegCompressor {
    fn drop(&mut self) {
        unsafe {
            tj::tj3Free(self.output.cast());
            tj::tj3Destroy(self.handle);
        }
    }
}

struct AsyncShared {
    latest_frame: Mutex<Option<OwnedCapturedFrame>>,
    spare_frame: Mutex<Option<OwnedCapturedFrame>>,
    frame_available: Condvar,
    stopped: AtomicBool,
    captured_frames: AtomicU32,
    replaced_frames: AtomicU32,
    skipped_frames: AtomicU32,
    callback_copy_total_us: AtomicU64,
    callback_copy_max_us: AtomicU64,
}

struct AsyncCallbackContext {
    shared: Arc<AsyncShared>,
}

#[derive(Debug)]
struct OwnedCapturedFrame {
    pixel_format: u32,
    plane0: Vec<u8>,
    width: usize,
    height: usize,
    plane0_stride: usize,
    plane1: Vec<u8>,
    plane1_width: usize,
    plane1_height: usize,
    plane1_stride: usize,
    dirty: DirtySummary,
    dirty_rects: Vec<DirtyRect>,
}

impl OwnedCapturedFrame {
    fn copy_from_callback(
        &mut self,
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
        dirty: DirtySummary,
        dirty_rects: *const DirtyRect,
        dirty_rects_len: usize,
    ) {
        self.pixel_format = pixel_format;
        self.width = width;
        self.height = height;
        self.plane0_stride = plane0_stride;
        self.plane1_width = plane1_width;
        self.plane1_height = plane1_height;
        self.plane1_stride = plane1_stride;
        self.dirty = dirty;
        if dirty_rects.is_null() || dirty_rects_len == 0 {
            self.dirty_rects.clear();
        } else {
            self.dirty_rects.clear();
            self.dirty_rects.extend_from_slice(unsafe {
                std::slice::from_raw_parts(dirty_rects, dirty_rects_len)
            });
        }

        self.plane0.resize(plane0_byte_count, 0);
        self.plane0
            .copy_from_slice(unsafe { std::slice::from_raw_parts(plane0, plane0_byte_count) });

        if plane1.is_null() || plane1_byte_count == 0 {
            self.plane1.clear();
        } else {
            self.plane1.resize(plane1_byte_count, 0);
            self.plane1
                .copy_from_slice(unsafe { std::slice::from_raw_parts(plane1, plane1_byte_count) });
        }
    }

    fn as_captured_frame(&self) -> CapturedFrame {
        CapturedFrame {
            pixel_format: self.pixel_format,
            plane0: self.plane0.as_ptr(),
            plane0_byte_count: self.plane0.len(),
            width: self.width,
            height: self.height,
            plane0_stride: self.plane0_stride,
            plane1: self.plane1.as_ptr(),
            plane1_byte_count: self.plane1.len(),
            plane1_width: self.plane1_width,
            plane1_height: self.plane1_height,
            plane1_stride: self.plane1_stride,
            dirty: self.dirty,
            dirty_rects: self.dirty_rects.as_ptr(),
            dirty_rects_len: self.dirty_rects.len(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct DirtyRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct DirtySummary {
    rect_count: usize,
    min_x: usize,
    min_y: usize,
    width: usize,
    height: usize,
    area: u64,
}

impl DirtySummary {
    fn bbox_area(self) -> u64 {
        self.width
            .saturating_mul(self.height)
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn area_ratio(self, frame_width: usize, frame_height: usize) -> f64 {
        let total = frame_width.saturating_mul(frame_height);
        if total == 0 {
            return 0.0;
        }
        self.area as f64 / total as f64
    }

    fn bbox_ratio(self, frame_width: usize, frame_height: usize) -> f64 {
        let total = frame_width.saturating_mul(frame_height);
        if total == 0 {
            return 0.0;
        }
        self.bbox_area() as f64 / total as f64
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ProfileStats {
    frames: u32,
    convert: Duration,
    encode: Duration,
    packet: Duration,
    usb: Duration,
    usb_header: Duration,
    usb_data: Duration,
    total: Duration,
    max_convert: Duration,
    max_encode: Duration,
    max_packet: Duration,
    max_usb: Duration,
    max_usb_header: Duration,
    max_usb_data: Duration,
    max_total: Duration,
    dirty_rects: u64,
    dirty_area_ratio: f64,
    dirty_bbox_ratio: f64,
    dirty_full_frames: u32,
    max_dirty_rects: usize,
    max_dirty_area_ratio: f64,
    max_dirty_bbox_ratio: f64,
    dirty_probe_convert: Duration,
    dirty_probe_encode: Duration,
    max_dirty_probe_convert: Duration,
    max_dirty_probe_encode: Duration,
    dirty_probe_payload_bytes: u64,
    max_dirty_probe_payload_bytes: usize,
}

impl ProfileStats {
    fn push(&mut self, sample: ProfileSample) {
        self.frames += 1;
        self.convert += sample.convert;
        self.encode += sample.encode;
        self.packet += sample.packet;
        self.usb += sample.usb;
        self.usb_header += sample.usb_header;
        self.usb_data += sample.usb_data;
        self.total += sample.total;
        self.max_convert = self.max_convert.max(sample.convert);
        self.max_encode = self.max_encode.max(sample.encode);
        self.max_packet = self.max_packet.max(sample.packet);
        self.max_usb = self.max_usb.max(sample.usb);
        self.max_usb_header = self.max_usb_header.max(sample.usb_header);
        self.max_usb_data = self.max_usb_data.max(sample.usb_data);
        self.max_total = self.max_total.max(sample.total);
        self.dirty_rects = self.dirty_rects.saturating_add(sample.dirty_rects as u64);
        self.dirty_area_ratio += sample.dirty_area_ratio;
        self.dirty_bbox_ratio += sample.dirty_bbox_ratio;
        self.dirty_full_frames += u32::from(sample.dirty_bbox_ratio >= 0.95);
        self.max_dirty_rects = self.max_dirty_rects.max(sample.dirty_rects);
        self.max_dirty_area_ratio = self.max_dirty_area_ratio.max(sample.dirty_area_ratio);
        self.max_dirty_bbox_ratio = self.max_dirty_bbox_ratio.max(sample.dirty_bbox_ratio);
        self.dirty_probe_convert += sample.dirty_probe_convert;
        self.dirty_probe_encode += sample.dirty_probe_encode;
        self.max_dirty_probe_convert = self.max_dirty_probe_convert.max(sample.dirty_probe_convert);
        self.max_dirty_probe_encode = self.max_dirty_probe_encode.max(sample.dirty_probe_encode);
        self.dirty_probe_payload_bytes = self
            .dirty_probe_payload_bytes
            .saturating_add(sample.dirty_probe_payload_bytes as u64);
        self.max_dirty_probe_payload_bytes = self
            .max_dirty_probe_payload_bytes
            .max(sample.dirty_probe_payload_bytes);
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    fn summary(self) -> String {
        if self.frames == 0 {
            return "no profile samples".to_string();
        }

        format!(
            "profile frames={} avg_ms convert={:.2} encode={:.2} packet={:.2} usb={:.2} usb_h={:.2} usb_d={:.2} total={:.2} max_ms convert={:.2} encode={:.2} packet={:.2} usb={:.2} usb_h={:.2} usb_d={:.2} total={:.2} dirty avg_rects={:.1} avg_area={:.1}% avg_bbox={:.1}% max_rects={} max_area={:.1}% max_bbox={:.1}% fullish={} tile_probe avg_ms convert={:.2} encode={:.2} max_ms convert={:.2} encode={:.2} avg_payload={} max_payload={}",
            self.frames,
            avg_ms(self.convert, self.frames),
            avg_ms(self.encode, self.frames),
            avg_ms(self.packet, self.frames),
            avg_ms(self.usb, self.frames),
            avg_ms(self.usb_header, self.frames),
            avg_ms(self.usb_data, self.frames),
            avg_ms(self.total, self.frames),
            duration_ms(self.max_convert),
            duration_ms(self.max_encode),
            duration_ms(self.max_packet),
            duration_ms(self.max_usb),
            duration_ms(self.max_usb_header),
            duration_ms(self.max_usb_data),
            duration_ms(self.max_total),
            self.dirty_rects as f64 / f64::from(self.frames),
            self.dirty_area_ratio * 100.0 / f64::from(self.frames),
            self.dirty_bbox_ratio * 100.0 / f64::from(self.frames),
            self.max_dirty_rects,
            self.max_dirty_area_ratio * 100.0,
            self.max_dirty_bbox_ratio * 100.0,
            self.dirty_full_frames,
            avg_ms(self.dirty_probe_convert, self.frames),
            avg_ms(self.dirty_probe_encode, self.frames),
            duration_ms(self.max_dirty_probe_convert),
            duration_ms(self.max_dirty_probe_encode),
            self.dirty_probe_payload_bytes / u64::from(self.frames),
            self.max_dirty_probe_payload_bytes
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ProfileSample {
    convert: Duration,
    encode: Duration,
    packet: Duration,
    usb: Duration,
    usb_header: Duration,
    usb_data: Duration,
    total: Duration,
    dirty_rects: usize,
    dirty_area_ratio: f64,
    dirty_bbox_ratio: f64,
    dirty_probe_convert: Duration,
    dirty_probe_encode: Duration,
    dirty_probe_payload_bytes: usize,
    send_path: SendPath,
    type7_setup_ack: Option<InterruptWaitSummary>,
    type7_tile: Option<Type7TileProfile>,
    type7_full_reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SendPath {
    #[default]
    Unknown,
    FullJpeg,
    RawNv12,
    Type4Setup,
    Type4SetupOnly,
    Type4SetupAndType7,
    Type7Tile,
}

#[derive(Clone, Copy, Debug, Default)]
struct Type7TileProfile {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    area_ratio: f64,
    jpeg_quality: i32,
    plane0_addr: u32,
    plane1_addr: u32,
    plane2_addr: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct InterruptWaitSummary {
    packets: u32,
    fences: u32,
    matched_fences: u32,
    jpeg_errors: u32,
    target_data: u32,
    last_event: u8,
    last_data: u32,
}

impl InterruptWaitSummary {
    fn ack_lag(self) -> u32 {
        self.target_data.saturating_sub(self.last_data)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct UsbTiming {
    header: Duration,
    data: Duration,
}

impl UsbTiming {
    fn total(self) -> Duration {
        self.header + self.data
    }
}

fn avg_ms(duration: Duration, frames: u32) -> f64 {
    duration.as_secs_f64() * 1000.0 / f64::from(frames)
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
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
    if options.reset_jpeg_engine {
        device
            .as_ref()
            .ok_or("--reset-jpeg-engine cannot be used with --dry-run")?
            .reset_jpeg_engine(u16::from(options.display_index))?;
        println!("Sent JPEG engine reset.");
    }
    if options.wait_interrupt_ms > 0 {
        let drained = drain_display_interrupts(
            device
                .as_ref()
                .ok_or("--wait-interrupt-ms cannot be used with --dry-run")?,
            Duration::from_millis(options.wait_interrupt_ms.max(50)),
        )?;
        if drained > 0 {
            println!("Drained {drained} pending display interrupts.");
        }
    }

    if options.async_send {
        return run_async(options, device, ram_size_mb);
    }

    run_sync(options, device, ram_size_mb)
}

fn build_sender_state(
    options: Options,
    device: Option<T6Device>,
    ram_size_mb: u8,
) -> Result<SenderState, Box<dyn Error>> {
    let layout = VramLayout::two_port_1080p_secondary(ram_size_mb);
    let jpeg_compressor = if matches!(options.transport, Transport::Jpeg | Transport::JpegType7)
        && matches!(
            options.jpeg_encoder,
            JpegEncoder::TurboBgra
                | JpegEncoder::T6Yuv420
                | JpegEncoder::VideoToolboxFullTurboTiles
        ) {
        Some(FastJpegCompressor::new(
            options.quality,
            options.subsamp.turbojpeg(),
        )?)
    } else {
        None
    };
    let vt_jpeg_compressor = if matches!(options.transport, Transport::Jpeg | Transport::JpegType7)
        && matches!(
            options.jpeg_encoder,
            JpegEncoder::VideoToolbox | JpegEncoder::VideoToolboxFullTurboTiles
        ) {
        let (output_width, output_height) =
            options.rotate.output_size(options.width, options.height);
        Some(VtJpegCompressor::new(
            usize::from(output_width),
            usize::from(output_height),
            options.quality,
        )?)
    } else {
        None
    };
    let current_quality = options.quality;
    let remaining_interrupt_dumps = options.dump_interrupts;
    let mut scheduler = FrameScheduler::new(layout);
    if options.transport == Transport::JpegType7 {
        if let Some(cmd_addr) = options.type7_cmd_addr {
            scheduler.set_cmd_ring(
                cmd_addr,
                options
                    .type7_cmd_wrap_addr
                    .unwrap_or_else(|| u32::from(ram_size_mb) * 1024 * 1024),
            );
        }
    }

    Ok(SenderState {
        frame_interval: Duration::from_secs_f64(1.0 / f64::from(options.fps.max(1))),
        next_send_at: Instant::now(),
        started_at: Instant::now(),
        last_report_at: Instant::now(),
        last_report_sent_frames: 0,
        sent_frames: 0,
        dropped_frames: 0,
        throttled_frames: 0,
        busy_frames: 0,
        late_frames: 0,
        current_display_fb_addr: None,
        current_type7_slot: 0,
        current_type7_allocated_base: TYPE7_MODE6_ALLOCATED_BASES[0],
        last_type7_dirty_bbox: None,
        dumped_type7_packets: 0,
        profile_stats: ProfileStats::default(),
        first_frame_dumped: false,
        sending: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        options,
        device,
        scheduler,
        jpeg_compressor,
        vt_jpeg_compressor,
        vt_jpeg_tile_compressors: HashMap::new(),
        rgb_scratch: Vec::new(),
        bgra_scratch: Vec::new(),
        dirty_bgra_scratch: Vec::new(),
        y_plane_scratch: Vec::new(),
        u_plane_scratch: Vec::new(),
        v_plane_scratch: Vec::new(),
        current_quality,
        remaining_interrupt_dumps,
        next_fence_id: 1,
    })
}

fn run_sync(
    options: Options,
    device: Option<T6Device>,
    ram_size_mb: u8,
) -> Result<(), Box<dyn Error>> {
    let state = Box::new(build_sender_state(options, device, ram_size_mb)?);
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
        "Started virtual display id={} capture={}x{} output={}x{} rotate={:?} fps={} transport={:?} capture_format={:?} quality={} subsamp={:?} jpeg_encoder={:?} dry_run={}",
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
        unsafe { (*state_ptr).options.jpeg_encoder },
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
            "Sent {} frames, dropped {} frames throttled={} busy={} late={} in {:.3}s",
            state.sent_frames,
            state.dropped_frames,
            state.throttled_frames,
            state.busy_frames,
            state.late_frames,
            state.started_at.elapsed().as_secs_f64()
        );
    }

    Ok(())
}

fn run_async(
    options: Options,
    device: Option<T6Device>,
    ram_size_mb: u8,
) -> Result<(), Box<dyn Error>> {
    if options.dump_first_frame.is_some() {
        return Err("--dump-first-frame is not supported with --async-send yet".into());
    }

    let shared = Arc::new(AsyncShared {
        latest_frame: Mutex::new(None),
        spare_frame: Mutex::new(None),
        frame_available: Condvar::new(),
        stopped: AtomicBool::new(false),
        captured_frames: AtomicU32::new(0),
        replaced_frames: AtomicU32::new(0),
        skipped_frames: AtomicU32::new(0),
        callback_copy_total_us: AtomicU64::new(0),
        callback_copy_max_us: AtomicU64::new(0),
    });
    let sender_shared = Arc::clone(&shared);
    let sender_state = build_sender_state(options.clone(), device, ram_size_mb)?;
    let sender = thread::spawn(move || async_sender_loop(sender_state, sender_shared));

    let context = Box::new(AsyncCallbackContext {
        shared: Arc::clone(&shared),
    });
    let context_ptr = Box::into_raw(context);

    let display_id = unsafe {
        t6_vd_start(
            usize::from(options.width),
            usize::from(options.height),
            f64::from(options.fps),
            options.capture_format.pixel_format(),
            async_frame_callback,
            context_ptr.cast(),
        )
    };
    if display_id == 0 {
        shared.stopped.store(true, Ordering::Relaxed);
        shared.frame_available.notify_all();
        let _context = unsafe { Box::from_raw(context_ptr) };
        return Err(format!(
            "failed to create virtual display or display stream: {}",
            unsafe { last_virtual_display_error() }
        )
        .into());
    }

    let (output_width, output_height) = options.rotate.output_size(options.width, options.height);
    println!(
        "Started virtual display id={} capture={}x{} output={}x{} rotate={:?} fps={} transport={:?} capture_format={:?} quality={} subsamp={:?} jpeg_encoder={:?} async_send=true dry_run={}",
        display_id,
        options.width,
        options.height,
        output_width,
        output_height,
        options.rotate,
        options.fps,
        options.transport,
        options.capture_format,
        options.quality,
        options.subsamp,
        options.jpeg_encoder,
        options.dry_run
    );
    println!("Stop with Ctrl-C, or use --frames N for a bounded run.");

    while !shared.stopped.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
    }

    unsafe {
        t6_vd_stop();
    }
    shared.frame_available.notify_all();
    let _context = unsafe { Box::from_raw(context_ptr) };

    let state = sender
        .join()
        .map_err(|_| "async sender thread panicked")?
        .map_err(|error| format!("async sender thread failed: {error}"))?;
    let captured_frames = shared.captured_frames.load(Ordering::Relaxed);
    let callback_copy_total_us = shared.callback_copy_total_us.load(Ordering::Relaxed);
    let callback_copy_avg_ms = if captured_frames > 0 {
        callback_copy_total_us as f64 / f64::from(captured_frames) / 1000.0
    } else {
        0.0
    };
    println!(
        "Sent {} frames, captured {} frames, replaced {} frames, skipped {} frames, dropped {} frames throttled={} busy={} late={} callback_copy_avg_ms={:.2} callback_copy_max_ms={:.2} in {:.3}s",
        state.sent_frames,
        captured_frames,
        shared.replaced_frames.load(Ordering::Relaxed),
        shared.skipped_frames.load(Ordering::Relaxed),
        state.dropped_frames,
        state.throttled_frames,
        state.busy_frames,
        state.late_frames,
        callback_copy_avg_ms,
        shared.callback_copy_max_us.load(Ordering::Relaxed) as f64 / 1000.0,
        state.started_at.elapsed().as_secs_f64()
    );

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
    dirty_rect_count: usize,
    dirty_min_x: usize,
    dirty_min_y: usize,
    dirty_width: usize,
    dirty_height: usize,
    dirty_area: u64,
    dirty_rects: *const DirtyRect,
    dirty_rects_len: usize,
    user_data: *mut c_void,
) {
    if plane0.is_null() || user_data.is_null() {
        return;
    }

    let state = unsafe { &mut *(user_data.cast::<SenderState>()) };
    if state.stopped.load(Ordering::Relaxed) {
        return;
    }

    if state.options.frame_throttle && Instant::now() < state.next_send_at {
        state.dropped_frames = state.dropped_frames.saturating_add(1);
        state.throttled_frames = state.throttled_frames.saturating_add(1);
        return;
    }

    if state.sending.swap(true, Ordering::Acquire) {
        state.dropped_frames = state.dropped_frames.saturating_add(1);
        state.busy_frames = state.busy_frames.saturating_add(1);
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
            dirty: DirtySummary {
                rect_count: dirty_rect_count,
                min_x: dirty_min_x,
                min_y: dirty_min_y,
                width: dirty_width,
                height: dirty_height,
                area: dirty_area,
            },
            dirty_rects,
            dirty_rects_len,
        },
    );
    state.sending.store(false, Ordering::Release);
    if let Err(error) = result {
        eprintln!("virtual display frame error: {error}");
        state.stopped.store(true, Ordering::Relaxed);
    }
}

extern "C" fn async_frame_callback(
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
    dirty_rect_count: usize,
    dirty_min_x: usize,
    dirty_min_y: usize,
    dirty_width: usize,
    dirty_height: usize,
    dirty_area: u64,
    dirty_rects: *const DirtyRect,
    dirty_rects_len: usize,
    user_data: *mut c_void,
) {
    if plane0.is_null() || user_data.is_null() {
        return;
    }

    let context = unsafe { &*(user_data.cast::<AsyncCallbackContext>()) };
    if context.shared.stopped.load(Ordering::Relaxed) {
        return;
    }

    let latest = match context.shared.latest_frame.try_lock() {
        Ok(latest) => latest,
        Err(_) => {
            context
                .shared
                .skipped_frames
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if latest.is_some() {
        context
            .shared
            .skipped_frames
            .fetch_add(1, Ordering::Relaxed);
        return;
    }
    drop(latest);

    let copy_started = Instant::now();
    let mut frame = context
        .shared
        .spare_frame
        .try_lock()
        .ok()
        .and_then(|mut spare| spare.take())
        .unwrap_or_else(|| OwnedCapturedFrame {
            pixel_format,
            plane0: Vec::with_capacity(plane0_byte_count),
            width,
            height,
            plane0_stride,
            plane1: Vec::with_capacity(plane1_byte_count),
            plane1_width,
            plane1_height,
            plane1_stride,
            dirty: DirtySummary::default(),
            dirty_rects: Vec::new(),
        });
    let dirty = DirtySummary {
        rect_count: dirty_rect_count,
        min_x: dirty_min_x,
        min_y: dirty_min_y,
        width: dirty_width,
        height: dirty_height,
        area: dirty_area,
    };
    frame.copy_from_callback(
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
        dirty,
        dirty_rects,
        dirty_rects_len,
    );
    let copy_us = copy_started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    context
        .shared
        .callback_copy_total_us
        .fetch_add(copy_us, Ordering::Relaxed);
    update_atomic_max(&context.shared.callback_copy_max_us, copy_us);

    let mut latest = context
        .shared
        .latest_frame
        .lock()
        .expect("latest frame mutex");
    if latest.is_some() {
        context
            .shared
            .skipped_frames
            .fetch_add(1, Ordering::Relaxed);
        drop(latest);
        recycle_async_frame(&context.shared, frame);
        return;
    }
    *latest = Some(frame);
    context
        .shared
        .captured_frames
        .fetch_add(1, Ordering::Relaxed);
    context.shared.frame_available.notify_one();
}

fn async_sender_loop(
    mut state: SenderState,
    shared: Arc<AsyncShared>,
) -> Result<SenderState, String> {
    loop {
        let frame = {
            let mut latest = shared.latest_frame.lock().expect("latest frame mutex");
            while latest.is_none() && !shared.stopped.load(Ordering::Relaxed) {
                latest = shared
                    .frame_available
                    .wait(latest)
                    .expect("latest frame mutex");
            }
            if shared.stopped.load(Ordering::Relaxed) && latest.is_none() {
                break;
            }
            latest.take()
        };

        let Some(frame) = frame else {
            continue;
        };
        if let Some(frame_limit) = state.options.frames {
            if state.sent_frames >= frame_limit {
                state.stopped.store(true, Ordering::Relaxed);
                shared.stopped.store(true, Ordering::Relaxed);
                recycle_async_frame(&shared, frame);
                break;
            }
        }

        if let Err(error) = send_frame(&mut state, frame.as_captured_frame()) {
            state.stopped.store(true, Ordering::Relaxed);
            shared.stopped.store(true, Ordering::Relaxed);
            return Err(error.to_string());
        }
        recycle_async_frame(&shared, frame);
        if state.stopped.load(Ordering::Relaxed) {
            shared.stopped.store(true, Ordering::Relaxed);
            break;
        }
    }

    Ok(state)
}

fn recycle_async_frame(shared: &AsyncShared, frame: OwnedCapturedFrame) {
    if let Ok(mut spare) = shared.spare_frame.try_lock() {
        if spare.is_none() {
            *spare = Some(frame);
        }
    }
}

fn update_atomic_max(value: &AtomicU64, sample: u64) {
    let mut current = value.load(Ordering::Relaxed);
    while sample > current {
        match value.compare_exchange_weak(current, sample, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
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
    dirty: DirtySummary,
    dirty_rects: *const DirtyRect,
    dirty_rects_len: usize,
}

fn send_frame(state: &mut SenderState, frame: CapturedFrame) -> Result<(), Box<dyn Error>> {
    let frame_started = Instant::now();
    let mut profile = ProfileSample::default();
    let options = state.options.clone();
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
    if options.dirty_mode != DirtyMode::Off {
        profile.dirty_rects = frame.dirty.rect_count;
        profile.dirty_area_ratio = frame
            .dirty
            .area_ratio(expected_width, expected_height)
            .min(1.0);
        profile.dirty_bbox_ratio = frame
            .dirty
            .bbox_ratio(expected_width, expected_height)
            .min(1.0);
    }

    let (output_width, output_height) = options.rotate.output_size(options.width, options.height);
    if let Some(path) = &options.dump_first_frame {
        if !state.first_frame_dumped
            && state.started_at.elapsed()
                >= Duration::from_millis(options.dump_first_frame_delay_ms)
        {
            let rgb = copy_bgra_to_rgb(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
            );
            rgb.save(path)?;
            state.first_frame_dumped = true;
            println!(
                "Dumped first frame to {} after {:.3}s",
                path.display(),
                state.started_at.elapsed().as_secs_f64()
            );
        }
    }

    let (payload_bytes, chunks, cmd_addr, fb_addr, reset_jpeg, fence_id) = match options.transport {
        Transport::Jpeg => {
            ensure_bgra_capture(frame)?;
            if options.dirty_mode == DirtyMode::TileSend {
                run_dirty_tile_jpeg_probe(
                    state,
                    frame,
                    expected_width,
                    expected_height,
                    &mut profile,
                )?;
            }
            send_full_jpeg_frame(
                state,
                frame,
                expected_width,
                expected_height,
                output_width,
                output_height,
                frame_started,
                &mut profile,
            )?
        }
        Transport::JpegType7 => {
            ensure_bgra_capture(frame)?;
            send_jpeg_type7_frame(
                state,
                frame,
                expected_width,
                expected_height,
                output_width,
                output_height,
                frame_started,
                &mut profile,
            )?
        }
        Transport::Nv12Type7 => send_raw_nv12_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            &mut profile,
        )?,
        Transport::Nv12 => send_raw_nv12_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            &mut profile,
        )?,
        Transport::Rgb24 => {
            ensure_bgra_capture(frame)?;
            copy_bgra_to_rgb_bytes(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
                &mut state.rgb_scratch,
            );
            let addresses = state.scheduler.next_jpeg_frame(state.rgb_scratch.len());
            let fence_id = next_fence_id(state);
            let packet = RawFramePacket::rgb24_with_fence(
                options.display_index,
                &state.rgb_scratch,
                output_width,
                output_height,
                addresses.fb_addr,
                0,
                fence_id,
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
                fence_id,
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
            let fence_id = next_fence_id(state);
            let packet = RawFramePacket::yv12_with_fence(
                options.display_index,
                &yv12,
                output_width,
                output_height,
                addresses.fb_addr,
                0,
                fence_id,
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
                fence_id,
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
            let fence_id = next_fence_id(state);
            let packet = RawFramePacket::yuv444_with_fence(
                options.display_index,
                &yuv444,
                output_width,
                output_height,
                addresses.fb_addr,
                0,
                fence_id,
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
                fence_id,
            )
        }
    };
    if payload_bytes == 0 && chunks == 0 {
        return Ok(());
    }
    let interrupt_summary = if options.wait_interrupt_ms > 0 {
        if let Some(device) = &state.device {
            Some(wait_for_display_interrupts(
                device,
                Duration::from_millis(options.wait_interrupt_ms),
                &mut state.remaining_interrupt_dumps,
                fence_id,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    state.sent_frames += 1;
    profile.total = frame_started.elapsed();
    adapt_jpeg_quality(state, profile.total);
    resync_next_send_at_for_profile(state, Instant::now(), profile);
    if options.profile {
        state.profile_stats.push(profile);
    }

    if state.sent_frames == 1
        || (options.report_every > 0 && state.sent_frames % options.report_every == 0)
    {
        let now = Instant::now();
        let sent_delta = state
            .sent_frames
            .saturating_sub(state.last_report_sent_frames);
        let report_elapsed = now.duration_since(state.last_report_at).as_secs_f64();
        let sent_fps = if report_elapsed > 0.0 {
            f64::from(sent_delta) / report_elapsed
        } else {
            0.0
        };
        println!(
            "frame={} fps={:.1} dropped={} throttled={} busy={} late={} quality={} payload_bytes={} path={} cmd=0x{:08x} fb=0x{:08x} fence=0x{:08x} reset={} chunks={} dirty_rects={} dirty_area={:.1}% dirty_bbox={}x{}+{}+{} ({:.1}%) tile_probe_payload={} tile_probe_ms convert={:.2} encode={:.2}{}{}{}{}",
            state.sent_frames,
            sent_fps,
            state.dropped_frames,
            state.throttled_frames,
            state.busy_frames,
            state.late_frames,
            state.current_quality,
            payload_bytes,
            format_send_path(profile.send_path),
            cmd_addr,
            fb_addr,
            fence_id,
            reset_jpeg,
            chunks,
            frame.dirty.rect_count,
            profile.dirty_area_ratio * 100.0,
            frame.dirty.width,
            frame.dirty.height,
            frame.dirty.min_x,
            frame.dirty.min_y,
            profile.dirty_bbox_ratio * 100.0,
            profile.dirty_probe_payload_bytes,
            duration_ms(profile.dirty_probe_convert),
            duration_ms(profile.dirty_probe_encode),
            format_type7_tile_summary(profile.type7_tile),
            format_type7_full_reason(profile.type7_full_reason),
            format_setup_ack_summary(profile.type7_setup_ack),
            format_interrupt_summary(interrupt_summary)
        );
        state.last_report_at = now;
        state.last_report_sent_frames = state.sent_frames;
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

fn send_raw_nv12_frame(
    state: &mut SenderState,
    frame: CapturedFrame,
    expected_width: usize,
    expected_height: usize,
    output_width: u16,
    output_height: u16,
    profile: &mut ProfileSample,
) -> Result<(usize, usize, u32, u32, bool, u32), Box<dyn Error>> {
    let convert_started = Instant::now();
    let nv12 = frame_to_nv12_full(state, frame, expected_width, expected_height)?;
    profile.convert += convert_started.elapsed();
    let addresses = state.scheduler.next_jpeg_frame(nv12.len());
    let fence_id = next_fence_id(state);
    let packet_started = Instant::now();
    let packet = RawFramePacket::nv12_with_fence(
        state.options.display_index,
        &nv12,
        output_width,
        output_height,
        addresses.fb_addr,
        0,
        fence_id,
    );
    let chunks = packet.bulk_chunks(state.options.max_packet_size).len();
    profile.packet += packet_started.elapsed();
    profile.send_path = SendPath::RawNv12;
    if let Some(device) = &state.device {
        let usb_timing = send_raw_packet(
            device,
            &packet,
            state.options.max_packet_size,
            state.options.raw_bulk_mode,
            "nv12",
        )?;
        profile.usb_header = usb_timing.header;
        profile.usb_data = usb_timing.data;
        profile.usb = usb_timing.total();
    }
    Ok((
        packet.payload.len(),
        chunks,
        addresses.cmd_addr,
        addresses.fb_addr,
        false,
        fence_id,
    ))
}

fn frame_to_nv12_full(
    state: &mut SenderState,
    frame: CapturedFrame,
    expected_width: usize,
    expected_height: usize,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let options = state.options.clone();
    if frame.pixel_format == PIXEL_FORMAT_420F || frame.pixel_format == PIXEL_FORMAT_420V {
        return copy_captured_nv12(frame, options.rotate);
    }

    ensure_bgra_capture(frame)?;
    bgra_to_nv12(
        frame.plane0,
        expected_width,
        expected_height,
        frame.plane0_stride,
        options.rotate,
        options.chroma_mode,
        options.yuv_matrix,
        options.yuv_range,
    )
}

fn send_full_jpeg_frame(
    state: &mut SenderState,
    frame: CapturedFrame,
    expected_width: usize,
    expected_height: usize,
    output_width: u16,
    output_height: u16,
    frame_started: Instant,
    profile: &mut ProfileSample,
) -> Result<(usize, usize, u32, u32, bool, u32), Box<dyn Error>> {
    let options = state.options.clone();
    let convert_started = Instant::now();
    let (jpeg_pixels, jpeg_pitch) = if options.rotate == Rotation::Deg0 {
        (frame.plane0, frame.plane0_stride)
    } else {
        let (rotated_width, rotated_height, rotated_stride) = rotate_bgra_with_vimage(
            frame.plane0,
            expected_width,
            expected_height,
            frame.plane0_stride,
            options.rotate,
            &mut state.bgra_scratch,
        )?;
        debug_assert_eq!(rotated_width, usize::from(output_width));
        debug_assert_eq!(rotated_height, usize::from(output_height));
        (state.bgra_scratch.as_ptr(), rotated_stride)
    };
    profile.convert = convert_started.elapsed();
    let frame_interval = state.frame_interval;
    if options.drop_late_frames && frame_started.elapsed() > frame_interval * 2 {
        drop_late_frame(state, *profile, frame_started);
        return Ok((0, 0, 0, 0, false, 0));
    }
    let fence_id = next_fence_id(state);
    let encode_started = Instant::now();
    let jpeg = match options.jpeg_encoder {
        JpegEncoder::TurboBgra => {
            let compressor = state
                .jpeg_compressor
                .as_mut()
                .ok_or("JPEG compressor is not initialized")?;
            compressor.compress_bgra_ptr(
                jpeg_pixels,
                usize::from(output_width),
                jpeg_pitch,
                usize::from(output_height),
                state.current_quality,
            )?
        }
        JpegEncoder::T6Yuv420 => {
            let compressor = state
                .jpeg_compressor
                .as_mut()
                .ok_or("JPEG compressor is not initialized")?;
            compressor.compress_t6_yuv420_bgra_ptr(
                jpeg_pixels,
                usize::from(output_width),
                jpeg_pitch,
                usize::from(output_height),
                state.current_quality,
                &mut state.y_plane_scratch,
                &mut state.u_plane_scratch,
                &mut state.v_plane_scratch,
            )?
        }
        JpegEncoder::VideoToolbox | JpegEncoder::VideoToolboxFullTurboTiles => {
            let compressor = state
                .vt_jpeg_compressor
                .as_mut()
                .ok_or("VideoToolbox JPEG compressor is not initialized")?;
            compressor.compress_bgra_ptr(
                jpeg_pixels,
                usize::from(output_width),
                jpeg_pitch,
                usize::from(output_height),
                state.current_quality,
            )?
        }
    };
    let type7_setup_jpeg;
    let use_type4_setup = options.transport == Transport::JpegType7
        && options.unsafe_generated_type7
        && matches!(
            options.type7_setup_mode,
            Type7SetupMode::SyntheticType4 | Type7SetupMode::PairedType4Type7
        );
    let jpeg = if use_type4_setup {
        type7_setup_jpeg = type7_mode6_jpeg_compat(jpeg, options.type7_jpeg_component_ids)?;
        type7_setup_jpeg.as_slice()
    } else {
        jpeg
    };
    let jpeg_len = jpeg.len();
    profile.encode = encode_started.elapsed();
    if options.drop_late_frames && frame_started.elapsed() > frame_interval * 2 {
        drop_late_frame(state, *profile, frame_started);
        return Ok((0, 0, 0, 0, false, 0));
    }
    let packet_started = Instant::now();
    if use_type4_setup {
        let slot = TYPE7_SETUP_SLOT_ORDER[state.current_type7_slot];
        let allocated_base = TYPE7_MODE6_ALLOCATED_BASES[slot];
        let canvas_width = align_usize(usize::from(output_width), 16) as u16;
        let canvas_height = align_usize(usize::from(output_width), 16) as u16;
        let (base0_addr, base1_addr, base2_addr) = type7_mode6_slot_bases(
            allocated_base,
            usize::from(output_width),
            usize::from(output_height),
        );
        let addresses = state
            .scheduler
            .next_video_payload_exact(type7_mode6_cmd_offset(jpeg_len) as usize, allocated_base);
        state.current_display_fb_addr = Some(allocated_base);
        state.current_type7_allocated_base = allocated_base;
        state.current_type7_slot = (state.current_type7_slot + 1) % TYPE7_SETUP_SLOT_ORDER.len();
        let packet = Type4Mode6SetupPacket::new(
            jpeg,
            addresses.cmd_addr,
            fence_id,
            canvas_width,
            canvas_height,
            base0_addr,
            base1_addr,
            base2_addr,
        );
        let chunks = packet.bulk_chunks(options.max_packet_size);
        profile.send_path = SendPath::Type4Setup;
        profile.packet = packet_started.elapsed();
        if options.type7_setup_mode == Type7SetupMode::PairedType4Type7 {
            let type7_fence_id = next_fence_id(state);
            let tile_width = usize::from(output_width).min(64);
            let tile_height = usize::from(output_height).min(64);
            crop_bgra_bbox(
                jpeg_pixels,
                jpeg_pitch,
                0,
                0,
                tile_width,
                tile_height,
                &mut state.dirty_bgra_scratch,
            );
            let mut tile_jpeg = encode_bgra_jpeg_vec(
                state,
                state.dirty_bgra_scratch.as_ptr(),
                tile_width,
                tile_width * 4,
                tile_height,
                state.current_quality,
            )?;
            patch_jpeg_type7_mode6_compat(&mut tile_jpeg, options.type7_jpeg_component_ids)?;
            let type7_addresses = state.scheduler.next_video_payload_exact(
                type7_mode6_cmd_offset(tile_jpeg.len()) as usize,
                allocated_base,
            );
            let (plane0_addr, plane1_addr, plane2_addr) = type7_mode6_plane_addrs(
                allocated_base,
                0,
                0,
                usize::from(output_width),
                usize::from(output_height),
                usize::from(canvas_width),
                usize::from(canvas_height),
            );
            profile.type7_tile = Some(Type7TileProfile {
                x: 0,
                y: 0,
                width: tile_width,
                height: tile_height,
                area_ratio: (tile_width * tile_height) as f64
                    / (usize::from(output_width) * usize::from(output_height)).max(1) as f64,
                jpeg_quality: state.current_quality,
                plane0_addr,
                plane1_addr,
                plane2_addr,
            });
            let type7_packet = Type7JpegTilePacket::new(
                &tile_jpeg,
                type7_addresses.cmd_addr,
                type7_fence_id,
                tile_width as u16,
                tile_height as u16,
                canvas_width,
                canvas_height,
                plane0_addr,
                plane1_addr,
                plane2_addr,
            );
            let type7_chunks = type7_packet.bulk_chunks(options.max_packet_size);
            profile.send_path = SendPath::Type4SetupAndType7;
            if let Some(device) = &state.device {
                let setup_usb = send_chunks_with_context(device, &chunks, "type4-setup")?;
                if options.type7_wait_setup_ack_ms > 0 {
                    profile.type7_setup_ack = Some(wait_for_display_interrupts(
                        device,
                        Duration::from_millis(options.type7_wait_setup_ack_ms),
                        &mut state.remaining_interrupt_dumps,
                        fence_id,
                    )?);
                }
                let type7_usb = send_chunks_with_context(device, &type7_chunks, "jpeg-type7")?;
                profile.usb_header = setup_usb.header + type7_usb.header;
                profile.usb_data = setup_usb.data + type7_usb.data;
                profile.usb = profile.usb_header + profile.usb_data;
            }
            return Ok((
                packet.payload.len() + type7_packet.payload.len(),
                chunks.len() + type7_chunks.len(),
                type7_addresses.cmd_addr,
                allocated_base,
                false,
                type7_fence_id,
            ));
        }
        if let Some(device) = &state.device {
            let usb_timing = send_chunks_with_context(device, &chunks, "type4-setup")?;
            profile.usb_header = usb_timing.header;
            profile.usb_data = usb_timing.data;
            profile.usb = usb_timing.total();
        }
        return Ok((
            packet.payload.len(),
            chunks.len(),
            addresses.cmd_addr,
            allocated_base,
            false,
            fence_id,
        ));
    }

    let addresses = state.scheduler.next_jpeg_frame(jpeg_len);
    state.current_display_fb_addr = Some(addresses.fb_addr);
    let flags = if addresses.reset_jpeg {
        VIDEO_FLAG_RESET_JPEG
    } else {
        0
    };
    let packet = JpegFramePacket::new_with_target_format_and_fence(
        options.display_index,
        jpeg,
        output_width,
        output_height,
        addresses.cmd_addr,
        addresses.fb_addr,
        options.jpeg_target.video_color(),
        flags,
        fence_id,
    );
    let chunks = packet.bulk_chunks(options.max_packet_size).len();
    profile.send_path = SendPath::FullJpeg;
    profile.packet = packet_started.elapsed();
    if let Some(device) = &state.device {
        let usb_timing =
            send_chunks_with_context(device, &packet.bulk_chunks(options.max_packet_size), "jpeg")?;
        profile.usb_header = usb_timing.header;
        profile.usb_data = usb_timing.data;
        profile.usb = usb_timing.total();
    }
    if options.dirty_mode == DirtyMode::Bbox {
        run_dirty_tile_jpeg_probe(state, frame, expected_width, expected_height, profile)?;
    }
    Ok((
        jpeg_len,
        chunks,
        addresses.cmd_addr,
        addresses.fb_addr,
        addresses.reset_jpeg,
        fence_id,
    ))
}

fn send_jpeg_type7_frame(
    state: &mut SenderState,
    frame: CapturedFrame,
    expected_width: usize,
    expected_height: usize,
    output_width: u16,
    output_height: u16,
    frame_started: Instant,
    profile: &mut ProfileSample,
) -> Result<(usize, usize, u32, u32, bool, u32), Box<dyn Error>> {
    if !state.options.unsafe_generated_type7 {
        return send_full_jpeg_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            frame_started,
            profile,
        );
    }
    if state.options.type7_setup_mode == Type7SetupMode::LegacyFullJpeg {
        return send_full_jpeg_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            frame_started,
            profile,
        );
    }

    let current_dirty_bbox = clamped_dirty_bbox(frame.dirty, expected_width, expected_height);
    let dirty_bbox = current_dirty_bbox.map(|bbox| match state.last_type7_dirty_bbox {
        Some(previous_bbox) => union_bbox(bbox, previous_bbox, expected_width, expected_height),
        None => bbox,
    });
    let fullish = profile.dirty_bbox_ratio >= 0.70;
    let refresh_frame = state.sent_frames == 0
        || (state.options.type7_refresh_frames > 0
            && state
                .sent_frames
                .is_multiple_of(state.options.type7_refresh_frames));
    if state.current_display_fb_addr.is_none() || dirty_bbox.is_none() || fullish || refresh_frame {
        profile.type7_full_reason = if state.current_display_fb_addr.is_none() {
            Some("no-current-fb")
        } else if dirty_bbox.is_none() {
            Some("no-dirty-bbox")
        } else if fullish {
            Some("fullish")
        } else {
            Some("refresh")
        };
        state.last_type7_dirty_bbox = None;
        return send_full_jpeg_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            frame_started,
            profile,
        );
    }

    let (dirty_x, dirty_y, dirty_width, dirty_height) = dirty_bbox.expect("checked above");
    state.last_type7_dirty_bbox = current_dirty_bbox;
    let (out_x, out_y, out_width, out_height) = rotate_bbox(
        dirty_x,
        dirty_y,
        dirty_width,
        dirty_height,
        expected_width,
        expected_height,
        state.options.rotate,
    );
    let out_w = usize::from(output_width);
    let out_h = usize::from(output_height);
    let Some((tile_x, tile_y, tile_width, tile_height)) = type7_update_rect(
        out_x,
        out_y,
        out_width,
        out_height,
        out_w,
        out_h,
        state.options.type7_rect_mode,
    ) else {
        return Ok((0, 0, 0, 0, false, 0));
    };
    let tile_ratio = (tile_width * tile_height) as f64 / (out_w * out_h).max(1) as f64;
    let too_small_horizontal_band =
        matches!(state.options.type7_rect_mode, Type7RectMode::HorizontalBand)
            && (tile_height as f64 / out_h.max(1) as f64) < state.options.type7_min_band_ratio;
    let too_small_vertical_band =
        matches!(state.options.type7_rect_mode, Type7RectMode::VerticalBand)
            && (tile_width as f64 / out_w.max(1) as f64) < state.options.type7_min_band_ratio;
    let too_small_local = matches!(state.options.type7_rect_mode, Type7RectMode::Local)
        && tile_ratio < state.options.type7_min_local_area_ratio;
    let too_small_band = too_small_horizontal_band || too_small_vertical_band;
    if tile_ratio >= state.options.type7_max_tile_ratio || too_small_band || too_small_local {
        profile.type7_full_reason = if tile_ratio >= state.options.type7_max_tile_ratio {
            Some("tile-ratio")
        } else if too_small_horizontal_band {
            Some("min-horizontal-band")
        } else if too_small_vertical_band {
            Some("min-vertical-band")
        } else {
            Some("min-local-area")
        };
        state.last_type7_dirty_bbox = None;
        return send_full_jpeg_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            frame_started,
            profile,
        );
    }

    let options = state.options.clone();
    let convert_started = Instant::now();
    crop_rotated_bgra_bbox(
        frame.plane0,
        expected_width,
        expected_height,
        frame.plane0_stride,
        options.rotate,
        tile_x,
        tile_y,
        tile_width,
        tile_height,
        &mut state.dirty_bgra_scratch,
    );
    profile.convert = convert_started.elapsed();
    let frame_interval = state.frame_interval;
    if options.drop_late_frames && frame_started.elapsed() > frame_interval * 2 {
        drop_late_frame(state, *profile, frame_started);
        return Ok((0, 0, 0, 0, false, 0));
    }

    let fence_id = next_fence_id(state);
    let tile_quality = type7_tile_quality(&options, state.current_quality, tile_ratio);
    let encode_started = Instant::now();
    let mut jpeg = encode_bgra_jpeg_vec(
        state,
        state.dirty_bgra_scratch.as_ptr(),
        tile_width,
        tile_width * 4,
        tile_height,
        tile_quality,
    )?;
    patch_jpeg_type7_mode6_compat(&mut jpeg, options.type7_jpeg_component_ids)?;
    let jpeg_len = jpeg.len();
    profile.encode = encode_started.elapsed();
    if options.drop_late_frames && frame_started.elapsed() > frame_interval * 2 {
        drop_late_frame(state, *profile, frame_started);
        return Ok((0, 0, 0, 0, false, 0));
    }

    let packet_started = Instant::now();
    if options.type7_setup_only {
        let setup_fence_id = fence_id;
        let type7_fence_id = next_fence_id(state);
        let slot = TYPE7_SETUP_SLOT_ORDER[state.current_type7_slot];
        let allocated_base = TYPE7_MODE6_ALLOCATED_BASES[slot];
        state.current_type7_slot = (state.current_type7_slot + 1) % TYPE7_SETUP_SLOT_ORDER.len();
        state.current_type7_allocated_base = allocated_base;
        state.current_display_fb_addr = Some(allocated_base);

        let full_convert_started = Instant::now();
        let (full_pixels, full_pitch) = if options.rotate == Rotation::Deg0 {
            (frame.plane0, frame.plane0_stride)
        } else {
            let (rotated_width, rotated_height, rotated_stride) = rotate_bgra_with_vimage(
                frame.plane0,
                expected_width,
                expected_height,
                frame.plane0_stride,
                options.rotate,
                &mut state.bgra_scratch,
            )?;
            debug_assert_eq!(rotated_width, out_w);
            debug_assert_eq!(rotated_height, out_h);
            (state.bgra_scratch.as_ptr(), rotated_stride)
        };
        profile.convert += full_convert_started.elapsed();

        let full_encode_started = Instant::now();
        let mut full_jpeg = encode_full_bgra_jpeg_vec(
            state,
            full_pixels,
            out_w,
            full_pitch,
            out_h,
            state.current_quality,
        )?;
        patch_jpeg_type7_mode6_compat(&mut full_jpeg, options.type7_jpeg_component_ids)?;
        profile.encode += full_encode_started.elapsed();

        let canvas_width = align_usize(out_w, 16);
        let canvas_height = align_usize(out_w, 16);
        let (base0_addr, base1_addr, base2_addr) =
            type7_mode6_slot_bases(allocated_base, out_w, out_h);
        let setup_addresses = state.scheduler.next_video_payload_exact(
            type7_mode6_cmd_offset(full_jpeg.len()) as usize,
            allocated_base,
        );
        let setup_packet = Type4Mode6SetupPacket::new(
            &full_jpeg,
            setup_addresses.cmd_addr,
            setup_fence_id,
            canvas_width as u16,
            canvas_height as u16,
            base0_addr,
            base1_addr,
            base2_addr,
        );
        let setup_chunks = setup_packet.bulk_chunks(options.max_packet_size);

        let (plane0_addr, plane1_addr, plane2_addr) = type7_mode6_plane_addrs(
            allocated_base,
            tile_x,
            tile_y,
            out_w,
            out_h,
            canvas_width,
            canvas_height,
        );
        profile.type7_tile = Some(Type7TileProfile {
            x: tile_x,
            y: tile_y,
            width: tile_width,
            height: tile_height,
            area_ratio: tile_ratio,
            jpeg_quality: state.current_quality,
            plane0_addr,
            plane1_addr,
            plane2_addr,
        });
        let type7_addresses = state
            .scheduler
            .next_video_payload_exact(type7_mode6_cmd_offset(jpeg_len) as usize, allocated_base);
        let type7_packet = Type7JpegTilePacket::new(
            &jpeg,
            type7_addresses.cmd_addr,
            type7_fence_id,
            tile_width as u16,
            tile_height as u16,
            canvas_width as u16,
            canvas_height as u16,
            plane0_addr,
            plane1_addr,
            plane2_addr,
        );
        let type7_chunks = type7_packet.bulk_chunks(options.max_packet_size);
        dump_type7_pair_packets(
            state,
            &setup_packet,
            &setup_chunks,
            setup_fence_id,
            &type7_packet,
            &type7_chunks,
            type7_fence_id,
        )?;
        profile.send_path = if options.type7_setup_only {
            SendPath::Type4SetupOnly
        } else {
            SendPath::Type4SetupAndType7
        };
        profile.packet = packet_started.elapsed();
        if options.type7_setup_only {
            if let Some(device) = &state.device {
                let setup_usb = send_chunks_with_context(device, &setup_chunks, "type4-setup")?;
                profile.usb_header = setup_usb.header;
                profile.usb_data = setup_usb.data;
                profile.usb = setup_usb.total();
            }
            return Ok((
                setup_packet.payload.len(),
                setup_chunks.len(),
                setup_addresses.cmd_addr,
                allocated_base,
                false,
                setup_fence_id,
            ));
        }
        if let Some(device) = &state.device {
            let setup_usb = send_chunks_with_context(device, &setup_chunks, "type4-setup")?;
            if options.type7_wait_setup_ack_ms > 0 {
                profile.type7_setup_ack = Some(wait_for_display_interrupts(
                    device,
                    Duration::from_millis(options.type7_wait_setup_ack_ms),
                    &mut state.remaining_interrupt_dumps,
                    setup_fence_id,
                )?);
            }
            let type7_usb = send_chunks_with_context(device, &type7_chunks, "jpeg-type7")?;
            profile.usb_header = setup_usb.header + type7_usb.header;
            profile.usb_data = setup_usb.data + type7_usb.data;
            profile.usb = profile.usb_header + profile.usb_data;
        }
        return Ok((
            setup_packet.payload.len() + type7_packet.payload.len(),
            setup_chunks.len() + type7_chunks.len(),
            type7_addresses.cmd_addr,
            allocated_base,
            false,
            type7_fence_id,
        ));
    }

    let fb_addr = state
        .current_display_fb_addr
        .ok_or("type7 has no current framebuffer address")?;
    let canvas_width = align_usize(out_w, 16);
    let canvas_height = align_usize(out_w, 16);
    let (plane0_addr, plane1_addr, plane2_addr) = type7_mode6_plane_addrs(
        state.current_type7_allocated_base,
        tile_x,
        tile_y,
        out_w,
        out_h,
        canvas_width,
        canvas_height,
    );
    profile.type7_tile = Some(Type7TileProfile {
        x: tile_x,
        y: tile_y,
        width: tile_width,
        height: tile_height,
        area_ratio: tile_ratio,
        jpeg_quality: tile_quality,
        plane0_addr,
        plane1_addr,
        plane2_addr,
    });
    let addresses = match state.options.type7_ring_mode {
        Type7RingMode::Windows => state.scheduler.next_type7_jpeg_payload(jpeg_len, fb_addr),
        Type7RingMode::Legacy => state
            .scheduler
            .next_jpeg_payload(48usize.saturating_add(jpeg_len), fb_addr),
    };
    if addresses.reset_jpeg {
        profile.type7_full_reason = Some("cmd-wrap");
        state.last_type7_dirty_bbox = None;
        return send_full_jpeg_frame(
            state,
            frame,
            expected_width,
            expected_height,
            output_width,
            output_height,
            frame_started,
            profile,
        );
    }
    let packet = Type7JpegTilePacket::new(
        &jpeg,
        addresses.cmd_addr,
        fence_id,
        tile_width as u16,
        tile_height as u16,
        canvas_width as u16,
        canvas_height as u16,
        plane0_addr,
        plane1_addr,
        plane2_addr,
    );
    let chunks = packet.bulk_chunks(options.max_packet_size);
    profile.send_path = SendPath::Type7Tile;
    dump_type7_packet(
        state,
        "type7",
        &packet.payload,
        addresses.cmd_addr,
        fence_id,
        &chunks,
    )?;
    profile.packet = packet_started.elapsed();
    if let Some(device) = &state.device {
        let usb_timing = send_chunks_with_context(device, &chunks, "jpeg-type7")?;
        profile.usb_header = usb_timing.header;
        profile.usb_data = usb_timing.data;
        profile.usb = usb_timing.total();
    }
    profile.dirty_probe_payload_bytes = jpeg_len;
    Ok((
        packet.payload.len(),
        chunks.len(),
        addresses.cmd_addr,
        fb_addr,
        false,
        fence_id,
    ))
}

fn encode_bgra_jpeg_vec(
    state: &mut SenderState,
    bgra: *const u8,
    width: usize,
    pitch: usize,
    height: usize,
    quality: i32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    match state.options.jpeg_encoder {
        JpegEncoder::TurboBgra | JpegEncoder::VideoToolboxFullTurboTiles => {
            let compressor = state
                .jpeg_compressor
                .as_mut()
                .ok_or("JPEG compressor is not initialized")?;
            Ok(compressor
                .compress_bgra_ptr(bgra, width, pitch, height, quality)?
                .to_vec())
        }
        JpegEncoder::T6Yuv420 => {
            let compressor = state
                .jpeg_compressor
                .as_mut()
                .ok_or("JPEG compressor is not initialized")?;
            Ok(compressor
                .compress_t6_yuv420_bgra_ptr(
                    bgra,
                    width,
                    pitch,
                    height,
                    quality,
                    &mut state.y_plane_scratch,
                    &mut state.u_plane_scratch,
                    &mut state.v_plane_scratch,
                )?
                .to_vec())
        }
        JpegEncoder::VideoToolbox => {
            let compressor = vt_jpeg_compressor_for_size(state, width, height, quality)?;
            Ok(compressor
                .compress_bgra_ptr(bgra, width, pitch, height, quality)?
                .to_vec())
        }
    }
}

fn encode_full_bgra_jpeg_vec(
    state: &mut SenderState,
    bgra: *const u8,
    width: usize,
    pitch: usize,
    height: usize,
    quality: i32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    match state.options.jpeg_encoder {
        JpegEncoder::VideoToolboxFullTurboTiles => {
            let compressor = state
                .vt_jpeg_compressor
                .as_mut()
                .ok_or("VideoToolbox JPEG compressor is not initialized")?;
            Ok(compressor
                .compress_bgra_ptr(bgra, width, pitch, height, quality)?
                .to_vec())
        }
        _ => encode_bgra_jpeg_vec(state, bgra, width, pitch, height, quality),
    }
}

fn vt_jpeg_compressor_for_size(
    state: &mut SenderState,
    width: usize,
    height: usize,
    quality: i32,
) -> Result<&mut VtJpegCompressor, Box<dyn Error>> {
    if let Some(compressor) = state.vt_jpeg_compressor.as_mut() {
        if compressor.width == width && compressor.height == height {
            return Ok(compressor);
        }
    }

    let key = (width, height);
    if !state.vt_jpeg_tile_compressors.contains_key(&key) {
        state
            .vt_jpeg_tile_compressors
            .insert(key, VtJpegCompressor::new(width, height, quality)?);
    }
    state
        .vt_jpeg_tile_compressors
        .get_mut(&key)
        .ok_or_else(|| "VideoToolbox JPEG tile compressor is not initialized".into())
}

fn type7_mode6_jpeg_compat(
    jpeg: &[u8],
    component_ids: Type7JpegComponentIds,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut patched = jpeg.to_vec();
    patch_jpeg_type7_mode6_compat(&mut patched, component_ids)?;
    Ok(patched)
}

fn patch_jpeg_type7_mode6_compat(
    jpeg: &mut Vec<u8>,
    component_ids: Type7JpegComponentIds,
) -> Result<(), Box<dyn Error>> {
    if component_ids == Type7JpegComponentIds::ZeroBased {
        patch_jpeg_component_ids_zero_based(jpeg)?;
    }
    patch_jfif_version_102(jpeg)?;
    insert_intel_ipp_comment(jpeg)?;
    Ok(())
}

fn patch_jfif_version_102(jpeg: &mut [u8]) -> Result<(), Box<dyn Error>> {
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
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if index + 2 > jpeg.len() {
            break;
        }
        let segment_len = u16::from_be_bytes([jpeg[index], jpeg[index + 1]]) as usize;
        if segment_len < 2 || index + segment_len > jpeg.len() {
            break;
        }
        let payload_start = index + 2;
        if marker == 0xe0 && segment_len >= 14 && jpeg[payload_start..].starts_with(b"JFIF\0") {
            jpeg[payload_start + 5] = 0x01;
            jpeg[payload_start + 6] = 0x02;
            break;
        }
        index += segment_len;
    }

    Ok(())
}

fn insert_intel_ipp_comment(jpeg: &mut Vec<u8>) -> Result<(), Box<dyn Error>> {
    const COMMENT: &[u8] = b"Intel(R) IPP JPEG encoder [7.1.37466] - Sep 25 2012;\0";
    if jpeg.len() < 4 || jpeg[0] != 0xff || jpeg[1] != 0xd8 {
        return Err("input is not a JPEG file".into());
    }
    if jpeg.windows(COMMENT.len()).any(|window| window == COMMENT) {
        return Ok(());
    }

    let mut insert_at = 2;
    if jpeg.len() >= 6 && jpeg[2] == 0xff && jpeg[3] == 0xe0 {
        let segment_len = u16::from_be_bytes([jpeg[4], jpeg[5]]) as usize;
        if segment_len >= 2 && 4 + segment_len <= jpeg.len() {
            insert_at = 4 + segment_len;
        }
    }

    let segment_len = COMMENT.len() + 2;
    if segment_len > u16::MAX as usize {
        return Err("JPEG comment is too large".into());
    }
    let mut segment = Vec::with_capacity(COMMENT.len() + 4);
    segment.extend_from_slice(&[0xff, 0xfe]);
    segment.extend_from_slice(&(segment_len as u16).to_be_bytes());
    segment.extend_from_slice(COMMENT);
    jpeg.splice(insert_at..insert_at, segment);
    Ok(())
}

fn patch_jpeg_component_ids_zero_based(jpeg: &mut [u8]) -> Result<(), Box<dyn Error>> {
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
            if segment_len < 8 {
                return Err("invalid JPEG SOF segment".into());
            }
            let component_count = jpeg[index + 7] as usize;
            let component_table = index + 8;
            if component_table + component_count * 3 > index + segment_len {
                return Err("invalid JPEG SOF component table".into());
            }
            for component in 0..component_count.min(3) {
                jpeg[component_table + component * 3] = component as u8;
            }
        } else if marker == 0xda {
            if segment_len < 4 {
                return Err("invalid JPEG SOS segment".into());
            }
            let component_count = jpeg[index + 2] as usize;
            let component_table = index + 3;
            if component_table + component_count * 2 > index + segment_len {
                return Err("invalid JPEG SOS component table".into());
            }
            for component in 0..component_count.min(3) {
                jpeg[component_table + component * 2] = component as u8;
            }
            break;
        }

        index += segment_len;
    }

    Ok(())
}

fn dump_type7_pair_packets(
    state: &mut SenderState,
    setup_packet: &Type4Mode6SetupPacket,
    setup_chunks: &[BulkTransferChunk<'_>],
    setup_fence_id: u32,
    type7_packet: &Type7JpegTilePacket,
    type7_chunks: &[BulkTransferChunk<'_>],
    type7_fence_id: u32,
) -> Result<(), Box<dyn Error>> {
    dump_type7_packet(
        state,
        "setup",
        &setup_packet.payload,
        setup_packet.payload_address,
        setup_fence_id,
        setup_chunks,
    )?;
    dump_type7_packet(
        state,
        "type7",
        &type7_packet.payload,
        type7_packet.payload_address,
        type7_fence_id,
        type7_chunks,
    )
}

fn dump_type7_packet(
    state: &mut SenderState,
    label: &str,
    payload: &[u8],
    payload_address: u32,
    fence_id: u32,
    chunks: &[BulkTransferChunk<'_>],
) -> Result<(), Box<dyn Error>> {
    let Some(dir) = state.options.dump_type7_packets.clone() else {
        return Ok(());
    };

    fs::create_dir_all(&dir)?;
    let index = state.dumped_type7_packets;
    state.dumped_type7_packets = state.dumped_type7_packets.saturating_add(1);
    let stem = format!("{index:06}_{label}_seq{fence_id:08x}");
    fs::write(dir.join(format!("{stem}.bin")), payload)?;

    let mut meta = String::new();
    meta.push_str(&format!("label={label}\n"));
    meta.push_str(&format!("sequence=0x{fence_id:08x}\n"));
    meta.push_str(&format!("payload_address=0x{payload_address:08x}\n"));
    meta.push_str(&format!("payload_len={}\n", payload.len()));
    meta.push_str(&format!("chunks={}\n", chunks.len()));
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        meta.push_str(&format!(
            "chunk[{chunk_index}] offset={} size={} more={} header_payload_len={} header_addr=0x{:08x}\n",
            chunk.header.packet_offset,
            chunk.header.packet_size,
            chunk.header.packet_flags != 0,
            chunk.header.payload_length,
            chunk.header.payload_address
        ));
    }
    if payload.len() >= 48 {
        meta.push_str(&format!(
            "video_type={} data_len={} word08=0x{:08x} word0c=0x{:08x}\n",
            u32::from_le_bytes(payload[0..4].try_into()?),
            u32::from_le_bytes(payload[4..8].try_into()?),
            u32::from_le_bytes(payload[8..12].try_into()?),
            u32::from_le_bytes(payload[12..16].try_into()?),
        ));
    }
    fs::write(dir.join(format!("{stem}.txt")), meta)?;

    Ok(())
}

fn run_dirty_tile_jpeg_probe(
    state: &mut SenderState,
    frame: CapturedFrame,
    frame_width: usize,
    frame_height: usize,
    profile: &mut ProfileSample,
) -> Result<(), Box<dyn Error>> {
    if frame.pixel_format != PIXEL_FORMAT_BGRA || frame.dirty.rect_count == 0 {
        return Ok(());
    }

    let dirty_rects = captured_dirty_rects(frame);
    if dirty_rects.is_empty() {
        if let Some((x, y, width, height)) =
            clamped_dirty_bbox(frame.dirty, frame_width, frame_height)
        {
            encode_dirty_tile_probe(state, frame, x, y, width, height, profile)?;
        }
        return Ok(());
    }

    for rect in dirty_rects {
        let Some((x, y, width, height)) = clamped_dirty_rect(*rect, frame_width, frame_height)
        else {
            continue;
        };
        encode_dirty_tile_probe(state, frame, x, y, width, height, profile)?;
    }

    Ok(())
}

fn encode_dirty_tile_probe(
    state: &mut SenderState,
    frame: CapturedFrame,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    profile: &mut ProfileSample,
) -> Result<(), Box<dyn Error>> {
    let convert_started = Instant::now();
    crop_bgra_bbox(
        frame.plane0,
        frame.plane0_stride,
        x,
        y,
        width,
        height,
        &mut state.dirty_bgra_scratch,
    );
    profile.dirty_probe_convert += convert_started.elapsed();
    let encode_started = Instant::now();
    let jpeg = encode_bgra_jpeg_vec(
        state,
        state.dirty_bgra_scratch.as_ptr(),
        width,
        width * 4,
        height,
        state.current_quality,
    )?;
    profile.dirty_probe_payload_bytes =
        profile.dirty_probe_payload_bytes.saturating_add(jpeg.len());
    profile.dirty_probe_encode += encode_started.elapsed();

    Ok(())
}

fn captured_dirty_rects(frame: CapturedFrame) -> &'static [DirtyRect] {
    if frame.dirty_rects.is_null() || frame.dirty_rects_len == 0 {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(frame.dirty_rects, frame.dirty_rects_len) }
}

fn clamped_dirty_bbox(
    dirty: DirtySummary,
    frame_width: usize,
    frame_height: usize,
) -> Option<(usize, usize, usize, usize)> {
    if dirty.width == 0 || dirty.height == 0 || frame_width == 0 || frame_height == 0 {
        return None;
    }
    let x = dirty.min_x.min(frame_width);
    let y = dirty.min_y.min(frame_height);
    let max_x = dirty.min_x.saturating_add(dirty.width).min(frame_width);
    let max_y = dirty.min_y.saturating_add(dirty.height).min(frame_height);
    if max_x <= x || max_y <= y {
        return None;
    }
    Some((x, y, max_x - x, max_y - y))
}

fn union_bbox(
    a: (usize, usize, usize, usize),
    b: (usize, usize, usize, usize),
    frame_width: usize,
    frame_height: usize,
) -> (usize, usize, usize, usize) {
    let min_x = a.0.min(b.0).min(frame_width);
    let min_y = a.1.min(b.1).min(frame_height);
    let max_x =
        a.0.saturating_add(a.2)
            .max(b.0.saturating_add(b.2))
            .min(frame_width);
    let max_y =
        a.1.saturating_add(a.3)
            .max(b.1.saturating_add(b.3))
            .min(frame_height);
    (
        min_x,
        min_y,
        max_x.saturating_sub(min_x),
        max_y.saturating_sub(min_y),
    )
}

fn clamped_dirty_rect(
    rect: DirtyRect,
    frame_width: usize,
    frame_height: usize,
) -> Option<(usize, usize, usize, usize)> {
    if rect.width == 0 || rect.height == 0 || frame_width == 0 || frame_height == 0 {
        return None;
    }
    let x = rect.x.min(frame_width);
    let y = rect.y.min(frame_height);
    let max_x = rect.x.saturating_add(rect.width).min(frame_width);
    let max_y = rect.y.saturating_add(rect.height).min(frame_height);
    if max_x <= x || max_y <= y {
        return None;
    }
    Some((x, y, max_x - x, max_y - y))
}

fn crop_bgra_bbox(
    bgra: *const u8,
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    out: &mut Vec<u8>,
) {
    let row_bytes = width * 4;
    out.resize(row_bytes * height, 0);
    for row in 0..height {
        let src =
            unsafe { std::slice::from_raw_parts(bgra.add((y + row) * stride + x * 4), row_bytes) };
        let dst = &mut out[row * row_bytes..(row + 1) * row_bytes];
        dst.copy_from_slice(src);
    }
}

fn crop_rotated_bgra_bbox(
    bgra: *const u8,
    source_width: usize,
    source_height: usize,
    source_stride: usize,
    rotation: Rotation,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    out: &mut Vec<u8>,
) {
    let row_bytes = width * 4;
    out.resize(row_bytes * height, 0);

    if rotation == Rotation::Deg0 {
        crop_bgra_bbox(bgra, source_stride, x, y, width, height, out);
        return;
    }

    match rotation {
        Rotation::Deg0 => unreachable!(),
        Rotation::Deg90 => {
            for out_y in 0..height {
                let src_x = y + out_y;
                let dst_row = &mut out[out_y * row_bytes..(out_y + 1) * row_bytes];
                for out_x in 0..width {
                    let src_y = source_height - 1 - (x + out_x);
                    let src = unsafe {
                        std::slice::from_raw_parts(bgra.add(src_y * source_stride + src_x * 4), 4)
                    };
                    let dst = out_x * 4;
                    dst_row[dst..dst + 4].copy_from_slice(src);
                }
            }
        }
        Rotation::Deg180 => {
            for out_y in 0..height {
                let src_y = source_height - 1 - (y + out_y);
                let dst_row = &mut out[out_y * row_bytes..(out_y + 1) * row_bytes];
                for out_x in 0..width {
                    let src_x = source_width - 1 - (x + out_x);
                    let src = unsafe {
                        std::slice::from_raw_parts(bgra.add(src_y * source_stride + src_x * 4), 4)
                    };
                    let dst = out_x * 4;
                    dst_row[dst..dst + 4].copy_from_slice(src);
                }
            }
        }
        Rotation::Deg270 => {
            for out_y in 0..height {
                let src_x = source_width - 1 - (y + out_y);
                let dst_row = &mut out[out_y * row_bytes..(out_y + 1) * row_bytes];
                for out_x in 0..width {
                    let src_y = x + out_x;
                    let src = unsafe {
                        std::slice::from_raw_parts(bgra.add(src_y * source_stride + src_x * 4), 4)
                    };
                    let dst = out_x * 4;
                    dst_row[dst..dst + 4].copy_from_slice(src);
                }
            }
        }
    }
}

fn wait_for_display_interrupts(
    device: &T6Device,
    duration: Duration,
    remaining_dumps: &mut u32,
    target_data: u32,
) -> Result<InterruptWaitSummary, Box<dyn Error>> {
    let deadline = Instant::now() + duration;
    let mut summary = InterruptWaitSummary {
        target_data,
        ..InterruptWaitSummary::default()
    };

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let timeout = (deadline - now).min(Duration::from_millis(10));
        match device.read_interrupt_packet_timeout(timeout) {
            Ok(packet) => {
                if *remaining_dumps > 0 {
                    println!("interrupt_raw={}", hex_bytes(&packet));
                    *remaining_dumps -= 1;
                }
                let interrupt = t6proto::DisplayInterrupt::parse(&packet);
                summary.packets = summary.packets.saturating_add(1);
                summary.last_event = interrupt.display_event;
                summary.last_data = interrupt.display_data;
                if interrupt.has_fence_id {
                    summary.fences = summary.fences.saturating_add(1);
                    if interrupt.display_data == target_data {
                        summary.matched_fences = summary.matched_fences.saturating_add(1);
                        break;
                    }
                }
                if interrupt.has_jpeg_error {
                    summary.jpeg_errors = summary.jpeg_errors.saturating_add(1);
                }
            }
            Err(rusb::Error::Timeout) => break,
            Err(error) => return Err(format!("interrupt read error: {error}").into()),
        }
    }

    Ok(summary)
}

fn drain_display_interrupts(device: &T6Device, duration: Duration) -> Result<u32, Box<dyn Error>> {
    let deadline = Instant::now() + duration;
    let mut drained = 0u32;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let timeout = (deadline - now).min(Duration::from_millis(10));
        match device.read_interrupt_packet_timeout(timeout) {
            Ok(_) => drained = drained.saturating_add(1),
            Err(rusb::Error::Timeout) => break,
            Err(error) => return Err(format!("interrupt drain error: {error}").into()),
        }
    }

    Ok(drained)
}

fn format_interrupt_summary(summary: Option<InterruptWaitSummary>) -> String {
    match summary {
        Some(summary) => format!(
            " interrupts={} fences={} matched={} target_data=0x{:08x} ack_lag={} jpeg_errors={} last_event=0x{:02x} last_data=0x{:08x}",
            summary.packets,
            summary.fences,
            summary.matched_fences,
            summary.target_data,
            summary.ack_lag(),
            summary.jpeg_errors,
            summary.last_event,
            summary.last_data
        ),
        None => String::new(),
    }
}

fn format_type7_tile_summary(tile: Option<Type7TileProfile>) -> String {
    match tile {
        Some(tile) => format!(
            " type7_tile={}x{}+{}+{} tile_area={:.1}% tile_q={} plane0=0x{:08x} plane1=0x{:08x} plane2=0x{:08x}",
            tile.width,
            tile.height,
            tile.x,
            tile.y,
            tile.area_ratio * 100.0,
            tile.jpeg_quality,
            tile.plane0_addr,
            tile.plane1_addr,
            tile.plane2_addr
        ),
        None => String::new(),
    }
}

fn type7_tile_quality(options: &Options, current_quality: i32, tile_ratio: f64) -> i32 {
    match options.type7_large_tile_quality {
        Some(quality) if tile_ratio >= options.type7_large_tile_ratio => quality,
        _ => current_quality,
    }
}

fn format_type7_full_reason(reason: Option<&'static str>) -> String {
    match reason {
        Some(reason) => format!(" type7_full_reason={reason}"),
        None => String::new(),
    }
}

fn format_setup_ack_summary(summary: Option<InterruptWaitSummary>) -> String {
    match summary {
        Some(summary) => format!(
            " setup_ack_packets={} setup_ack_fences={} setup_ack_matched={} setup_ack_lag={} setup_ack_last=0x{:08x}",
            summary.packets,
            summary.fences,
            summary.matched_fences,
            summary.ack_lag(),
            summary.last_data
        ),
        None => String::new(),
    }
}

fn format_send_path(path: SendPath) -> &'static str {
    match path {
        SendPath::Unknown => "unknown",
        SendPath::FullJpeg => "full-jpeg",
        SendPath::RawNv12 => "raw-nv12",
        SendPath::Type4Setup => "type4-setup",
        SendPath::Type4SetupOnly => "type4-setup-only",
        SendPath::Type4SetupAndType7 => "type4+type7",
        SendPath::Type7Tile => "type7",
    }
}

fn next_fence_id(state: &mut SenderState) -> u32 {
    let fence_id = state.next_fence_id;
    state.next_fence_id = state.next_fence_id.wrapping_add(1).max(1);
    fence_id
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

fn drop_late_frame(state: &mut SenderState, mut profile: ProfileSample, frame_started: Instant) {
    state.dropped_frames = state.dropped_frames.saturating_add(1);
    state.late_frames = state.late_frames.saturating_add(1);
    profile.total = frame_started.elapsed();
    adapt_jpeg_quality(state, profile.total);
    resync_next_send_at(state, Instant::now());
    if state.options.profile {
        state.profile_stats.push(profile);
    }
}

fn resync_next_send_at(state: &mut SenderState, now: Instant) {
    resync_next_send_at_with_divisor(state, now, 1);
}

fn resync_next_send_at_for_profile(state: &mut SenderState, now: Instant, profile: ProfileSample) {
    let divisor = profile
        .type7_tile
        .filter(|tile| tile.area_ratio >= state.options.type7_large_tile_ratio)
        .map(|_| state.options.type7_large_tile_fps_divisor.max(1))
        .unwrap_or(1);
    resync_next_send_at_with_divisor(state, now, divisor);
}

fn resync_next_send_at_with_divisor(state: &mut SenderState, now: Instant, divisor: u32) {
    let interval = state.frame_interval * divisor;
    let next = state.next_send_at + interval;
    state.next_send_at = if now > next + interval { now } else { next };
}

fn adapt_jpeg_quality(state: &mut SenderState, total: Duration) {
    let options = &state.options;
    if !matches!(options.transport, Transport::Jpeg | Transport::JpegType7)
        || !options.adaptive_quality
    {
        return;
    }

    let budget = state.frame_interval;
    let next_quality = if total > budget * 8 {
        state.current_quality - 15
    } else if total > budget * 4 {
        state.current_quality - 10
    } else if total > budget * 2 {
        state.current_quality - 5
    } else if total > budget + budget / 2 {
        state.current_quality - 3
    } else if total > budget {
        state.current_quality - 1
    } else if total < budget * 3 / 4 {
        state.current_quality + 1
    } else {
        state.current_quality
    };

    state.current_quality = next_quality.clamp(options.min_quality, options.quality);
}

fn send_chunks_with_context(
    device: &T6Device,
    chunks: &[BulkTransferChunk<'_>],
    label: &str,
) -> Result<UsbTiming, Box<dyn Error>> {
    let mut timing = UsbTiming::default();
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let header_started = Instant::now();
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
        timing.header += header_started.elapsed();
        let data_started = Instant::now();
        device.write_display_bulk(chunk.data).map_err(|error| {
            format!(
                "{label} bulk data error at chunk {}/{} offset={} size={}: {error}",
                chunk_index + 1,
                chunks.len(),
                chunk.header.packet_offset,
                chunk.header.packet_size
            )
        })?;
        timing.data += data_started.elapsed();
    }

    Ok(timing)
}

fn send_raw_packet(
    device: &T6Device,
    packet: &RawFramePacket,
    max_packet_size: u32,
    mode: RawBulkMode,
    label: &str,
) -> Result<UsbTiming, Box<dyn Error>> {
    match mode {
        RawBulkMode::Fragmented => {
            send_chunks_with_context(device, &packet.bulk_chunks(max_packet_size), label)
        }
        RawBulkMode::Single => {
            let mut timing = UsbTiming::default();
            let header = BulkDmaHeader::display(
                packet.payload.len() as u32,
                packet.payload_address,
                packet.payload.len() as u32,
                0,
                false,
            );
            let header_started = Instant::now();
            device
                .write_display_bulk(&header.to_bytes())
                .map_err(|error| {
                    format!(
                        "{label} single bulk header error payload_bytes={} addr=0x{:08x}: {error}",
                        packet.payload.len(),
                        packet.payload_address
                    )
                })?;
            timing.header = header_started.elapsed();
            let data_started = Instant::now();
            device
                .write_display_bulk(&packet.payload)
                .map_err(|error| {
                    format!(
                        "{label} single bulk data error payload_bytes={} addr=0x{:08x}: {error}",
                        packet.payload.len(),
                        packet.payload_address
                    )
                })?;
            timing.data = data_started.elapsed();

            Ok(timing)
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

fn fill_t6_yuv420_from_bgra(
    bgra: *const u8,
    width: usize,
    pitch: usize,
    height: usize,
    y_plane: &mut Vec<u8>,
    u_plane: &mut Vec<u8>,
    v_plane: &mut Vec<u8>,
) {
    y_plane.resize(width * height, 0);
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    u_plane.resize(chroma_width * chroma_height, 0);
    v_plane.resize(chroma_width * chroma_height, 0);

    for y in 0..height {
        let row = unsafe { std::slice::from_raw_parts(bgra.add(y * pitch), width * 4) };
        for x in 0..width {
            let i = x * 4;
            let b = row[i] as f32;
            let g = row[i + 1] as f32;
            let r = row[i + 2] as f32;
            y_plane[y * width + x] = clamp_f32_u8((0.299 * r) + (0.587 * g) + (0.114 * b));
        }
    }

    for cy in 0..chroma_height {
        let row0 = unsafe { std::slice::from_raw_parts(bgra.add((cy * 2) * pitch), width * 4) };
        let row1 = unsafe { std::slice::from_raw_parts(bgra.add((cy * 2 + 1) * pitch), width * 4) };
        for cx in 0..chroma_width {
            let x = cx * 2;
            let i0 = x * 4;
            let i1 = (x + 1) * 4;
            let b_sum = row0[i0] as f32 + row0[i1] as f32 + row1[i0] as f32 + row1[i1] as f32;
            let g_sum = row0[i0 + 1] as f32
                + row0[i1 + 1] as f32
                + row1[i0 + 1] as f32
                + row1[i1 + 1] as f32;
            let r_sum = row0[i0 + 2] as f32
                + row0[i1 + 2] as f32
                + row1[i0 + 2] as f32
                + row1[i1 + 2] as f32;
            let r = r_sum * 0.25;
            let g = g_sum * 0.25;
            let b = b_sum * 0.25;
            let u = (-0.168736 * r) - (0.331264 * g) + (0.5 * b) + 128.0;
            let v = (0.5 * r) - (0.418688 * g) - (0.081312 * b) + 128.0;
            let ci = cy * chroma_width + cx;
            u_plane[ci] = clamp_f32_u8(u);
            v_plane[ci] = clamp_f32_u8(v);
        }
    }
}

fn clamp_f32_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn align_usize(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn type7_update_rect(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    canvas_width: usize,
    canvas_height: usize,
    mode: Type7RectMode,
) -> Option<(usize, usize, usize, usize)> {
    let local = align_type7_rect_32(x, y, width, height, canvas_width, canvas_height)?;
    let horizontal = (0, local.1, canvas_width, local.3);
    let vertical = (local.0, 0, local.2, canvas_height);

    let selected = match mode {
        Type7RectMode::Local => local,
        Type7RectMode::HorizontalBand => horizontal,
        Type7RectMode::VerticalBand => vertical,
        Type7RectMode::Auto => {
            let local_area = local.2.saturating_mul(local.3);
            let horizontal_area = horizontal.2.saturating_mul(horizontal.3);
            let vertical_area = vertical.2.saturating_mul(vertical.3);
            if local_area <= horizontal_area && local_area <= vertical_area {
                local
            } else if horizontal_area <= vertical_area {
                horizontal
            } else {
                vertical
            }
        }
    };

    if selected.2 == 0 || selected.3 == 0 {
        None
    } else {
        Some(selected)
    }
}

fn align_type7_rect_32(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    canvas_width: usize,
    canvas_height: usize,
) -> Option<(usize, usize, usize, usize)> {
    if width == 0 || height == 0 || canvas_width == 0 || canvas_height == 0 {
        return None;
    }

    let mut left = x.saturating_sub(1) / 32 * 32;
    let mut top = y.saturating_sub(1) / 32 * 32;
    let mut right = align_usize(x.saturating_add(width).saturating_add(31), 32).min(canvas_width);
    let mut bottom =
        align_usize(y.saturating_add(height).saturating_add(31), 32).min(canvas_height);

    if right <= left || bottom <= top {
        return None;
    }

    if right - left < 32 {
        if right == canvas_width {
            left = left.saturating_sub(32 - (right - left));
        } else {
            right = (left + 32).min(canvas_width);
        }
    }
    if bottom - top < 32 {
        if bottom == canvas_height {
            top = top.saturating_sub(32 - (bottom - top));
        } else {
            bottom = (top + 32).min(canvas_height);
        }
    }

    Some((left, top, right - left, bottom - top))
}

fn type7_mode6_plane_addrs(
    allocated_base: u32,
    left: usize,
    top: usize,
    output_width: usize,
    output_height: usize,
    canvas_width: usize,
    _canvas_height: usize,
) -> (u32, u32, u32) {
    let (base0, base1, _) = type7_mode6_slot_bases(allocated_base, output_width, output_height);
    let plane0 = base0
        .saturating_add((canvas_width as u32).saturating_mul(top as u32))
        .saturating_add(left as u32);
    let plane1 = base1
        .saturating_add(((top / 2) as u32).saturating_mul(canvas_width as u32))
        .saturating_add(((left / 2) as u32).saturating_mul(2));

    (plane0, plane1, 0)
}

fn type7_mode6_slot_bases(
    allocated_base: u32,
    output_width: usize,
    output_height: usize,
) -> (u32, u32, u32) {
    let tile_blocks = output_width
        .div_ceil(16)
        .saturating_mul(output_height.div_ceil(16));
    let plane_span = (tile_blocks as u32).saturating_mul(0x100);
    let base0 = allocated_base.saturating_add(0x30);
    let base1 = base0.saturating_add(plane_span);
    (base0, base1, 0)
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
    let mut rgb = Vec::new();
    copy_bgra_to_rgb_bytes(bgra, width, height, stride, rotation, &mut rgb);

    RgbImage::from_raw(output_width as u32, output_height as u32, rgb)
        .expect("RGB buffer has expected size")
}

fn copy_bgra_to_rgb_bytes(
    bgra: *const u8,
    width: usize,
    height: usize,
    stride: usize,
    rotation: Rotation,
    rgb: &mut Vec<u8>,
) {
    let (output_width, _) = match rotation {
        Rotation::Deg0 | Rotation::Deg180 => (width, height),
        Rotation::Deg90 | Rotation::Deg270 => (height, width),
    };
    rgb.resize(width * height * 3, 0);

    match rotation {
        Rotation::Deg0 => {
            for y in 0..height {
                let row = unsafe { std::slice::from_raw_parts(bgra.add(y * stride), width * 4) };
                let out_row = &mut rgb[y * width * 3..(y + 1) * width * 3];
                for x in 0..width {
                    let src = x * 4;
                    let dst = x * 3;
                    out_row[dst] = row[src + 2];
                    out_row[dst + 1] = row[src + 1];
                    out_row[dst + 2] = row[src];
                }
            }
        }
        Rotation::Deg90 => {
            for out_y in 0..width {
                let out_row = &mut rgb[out_y * output_width * 3..(out_y + 1) * output_width * 3];
                for out_x in 0..height {
                    let src_y = height - 1 - out_x;
                    let src = out_y * 4;
                    let pixel =
                        unsafe { std::slice::from_raw_parts(bgra.add(src_y * stride + src), 4) };
                    let dst = out_x * 3;
                    out_row[dst] = pixel[2];
                    out_row[dst + 1] = pixel[1];
                    out_row[dst + 2] = pixel[0];
                }
            }
        }
        Rotation::Deg180 => {
            for src_y in 0..height {
                let row =
                    unsafe { std::slice::from_raw_parts(bgra.add(src_y * stride), width * 4) };
                let out_y = height - 1 - src_y;
                for src_x in 0..width {
                    let out_x = width - 1 - src_x;
                    let src = src_x * 4;
                    let dst = (out_y * output_width + out_x) * 3;
                    rgb[dst] = row[src + 2];
                    rgb[dst + 1] = row[src + 1];
                    rgb[dst + 2] = row[src];
                }
            }
        }
        Rotation::Deg270 => {
            for out_y in 0..width {
                let out_row = &mut rgb[out_y * output_width * 3..(out_y + 1) * output_width * 3];
                let src_x = width - 1 - out_y;
                for out_x in 0..height {
                    let pixel = unsafe {
                        std::slice::from_raw_parts(bgra.add(out_x * stride + src_x * 4), 4)
                    };
                    let dst = out_x * 3;
                    out_row[dst] = pixel[2];
                    out_row[dst + 1] = pixel[1];
                    out_row[dst + 2] = pixel[0];
                }
            }
        }
    }
}

fn rotate_bgra_with_vimage(
    bgra: *const u8,
    width: usize,
    height: usize,
    stride: usize,
    rotation: Rotation,
    out: &mut Vec<u8>,
) -> Result<(usize, usize, usize), Box<dyn Error>> {
    let (output_width, output_height) = match rotation {
        Rotation::Deg0 | Rotation::Deg180 => (width, height),
        Rotation::Deg90 | Rotation::Deg270 => (height, width),
    };
    let output_stride = output_width * 4;
    out.resize(output_stride * output_height, 0);

    let rotation_constant = match rotation {
        Rotation::Deg0 => 0,
        Rotation::Deg90 => 3,
        Rotation::Deg180 => 2,
        Rotation::Deg270 => 1,
    };
    let src = VImageBuffer {
        data: bgra.cast_mut().cast(),
        height,
        width,
        row_bytes: stride,
    };
    let dest = VImageBuffer {
        data: out.as_mut_ptr().cast(),
        height: output_height,
        width: output_width,
        row_bytes: output_stride,
    };
    let back_color = [0u8, 0, 0, 255];
    let status =
        unsafe { vImageRotate90_ARGB8888(&src, &dest, rotation_constant, back_color.as_ptr(), 0) };
    if status != 0 {
        return Err(format!("vImageRotate90_ARGB8888 failed: {status}").into());
    }

    Ok((output_width, output_height, output_stride))
}

fn rotate_bbox(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    frame_width: usize,
    frame_height: usize,
    rotation: Rotation,
) -> (usize, usize, usize, usize) {
    let x1 = x;
    let y1 = y;
    let x2 = x.saturating_add(width).saturating_sub(1);
    let y2 = y.saturating_add(height).saturating_sub(1);
    let corners = [
        output_coordinate(x1, y1, frame_width, frame_height, rotation),
        output_coordinate(x2, y1, frame_width, frame_height, rotation),
        output_coordinate(x1, y2, frame_width, frame_height, rotation),
        output_coordinate(x2, y2, frame_width, frame_height, rotation),
    ];
    let min_x = corners.iter().map(|(cx, _)| *cx).min().unwrap_or(0);
    let max_x = corners.iter().map(|(cx, _)| *cx).max().unwrap_or(min_x);
    let min_y = corners.iter().map(|(_, cy)| *cy).min().unwrap_or(0);
    let max_y = corners.iter().map(|(_, cy)| *cy).max().unwrap_or(min_y);
    (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

fn output_coordinate(
    src_x: usize,
    src_y: usize,
    width: usize,
    height: usize,
    rotation: Rotation,
) -> (usize, usize) {
    match rotation {
        Rotation::Deg0 => (src_x, src_y),
        Rotation::Deg90 => (height - 1 - src_y, src_x),
        Rotation::Deg180 => (width - 1 - src_x, height - 1 - src_y),
        Rotation::Deg270 => (src_y, width - 1 - src_x),
    }
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
    let mut width = 1080;
    let mut height = 1920;
    let mut rotate = Rotation::Deg90;
    let mut fps = 60;
    let mut frames = None;
    let mut quality = 95;
    let mut adaptive_quality = false;
    let mut min_quality = 85;
    let mut subsamp = JpegSubsampling::Yuv420;
    let mut jpeg_encoder = JpegEncoder::VideoToolboxFullTurboTiles;
    let mut jpeg_target = JpegTarget::Nv12;
    let mut chroma_mode = ChromaMode::Average;
    let mut yuv_matrix = YuvMatrix::Bt601;
    let mut yuv_range = YuvRange::Full;
    let mut transport = Transport::JpegType7;
    let mut capture_format = CaptureFormat::Bgra;
    let mut raw_bulk_mode = RawBulkMode::Fragmented;
    let mut ready = false;
    let mut power_on = false;
    let mut reset_jpeg_engine = false;
    let mut profile = false;
    let mut report_every = 60;
    let mut async_send = false;
    let mut frame_throttle = false;
    let mut drop_late_frames = true;
    let mut unsafe_generated_type7 = true;
    let mut type7_rect_mode = Type7RectMode::Local;
    let mut type7_ring_mode = Type7RingMode::Windows;
    let mut type7_refresh_frames = 0;
    let mut type7_min_band_ratio = 0.20;
    let mut type7_min_local_area_ratio = 0.0;
    let mut type7_max_tile_ratio = 1.0;
    let mut type7_large_tile_ratio = 0.25;
    let mut type7_large_tile_quality = Some(88);
    let mut type7_large_tile_fps_divisor = 2;
    let mut type7_setup_mode = Type7SetupMode::PairedType4Type7;
    let mut type7_jpeg_component_ids = Type7JpegComponentIds::Original;
    let mut type7_wait_setup_ack_ms = 0;
    let mut type7_setup_only = false;
    let mut dirty_mode = DirtyMode::Log;
    let mut dry_run = false;
    let mut ram_size_mb = None;
    let mut usb_timeout_ms = 3000;
    let mut wait_interrupt_ms = 0;
    let mut dump_interrupts = 0;
    let mut max_packet_size = DEFAULT_MAX_BULK_PACKET_SIZE;
    let mut dump_first_frame = None;
    let mut dump_first_frame_delay_ms = 0;
    let mut dump_type7_packets = None;
    let mut type7_cmd_addr = Some(0x0320_0000);
    let mut type7_cmd_wrap_addr = Some(0x0390_0000);
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
            "--adaptive-quality" => adaptive_quality = true,
            "--min-quality" => min_quality = next_value(&mut args, "--min-quality")?.parse()?,
            "--subsamp" => subsamp = parse_subsampling(&next_value(&mut args, "--subsamp")?)?,
            "--jpeg-encoder" => {
                jpeg_encoder = parse_jpeg_encoder(&next_value(&mut args, "--jpeg-encoder")?)?
            }
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
            "--reset-jpeg-engine" => reset_jpeg_engine = true,
            "--profile" => profile = true,
            "--report-every" => report_every = next_value(&mut args, "--report-every")?.parse()?,
            "--async-send" => async_send = true,
            "--frame-throttle" => frame_throttle = true,
            "--no-frame-throttle" => frame_throttle = false,
            "--drop-late-frames" => drop_late_frames = true,
            "--no-drop-late-frames" => drop_late_frames = false,
            "--unsafe-generated-type7" => unsafe_generated_type7 = true,
            "--no-unsafe-generated-type7" => unsafe_generated_type7 = false,
            "--type7-rect" => {
                type7_rect_mode = parse_type7_rect_mode(&next_value(&mut args, "--type7-rect")?)?
            }
            "--type7-ring" => {
                type7_ring_mode = parse_type7_ring_mode(&next_value(&mut args, "--type7-ring")?)?
            }
            "--type7-refresh-frames" => {
                type7_refresh_frames = next_value(&mut args, "--type7-refresh-frames")?.parse()?
            }
            "--type7-min-band-ratio" => {
                type7_min_band_ratio = next_value(&mut args, "--type7-min-band-ratio")?.parse()?
            }
            "--type7-min-local-area-ratio" => {
                type7_min_local_area_ratio =
                    next_value(&mut args, "--type7-min-local-area-ratio")?.parse()?
            }
            "--type7-max-tile-ratio" => {
                type7_max_tile_ratio = next_value(&mut args, "--type7-max-tile-ratio")?.parse()?
            }
            "--type7-large-tile-ratio" => {
                type7_large_tile_ratio =
                    next_value(&mut args, "--type7-large-tile-ratio")?.parse()?
            }
            "--type7-large-tile-quality" => {
                type7_large_tile_quality =
                    Some(next_value(&mut args, "--type7-large-tile-quality")?.parse()?)
            }
            "--type7-large-tile-fps-divisor" => {
                type7_large_tile_fps_divisor =
                    next_value(&mut args, "--type7-large-tile-fps-divisor")?.parse()?
            }
            "--type7-setup" => {
                type7_setup_mode = parse_type7_setup_mode(&next_value(&mut args, "--type7-setup")?)?
            }
            "--type7-jpeg-component-ids" => {
                type7_jpeg_component_ids = parse_type7_jpeg_component_ids(&next_value(
                    &mut args,
                    "--type7-jpeg-component-ids",
                )?)?
            }
            "--type7-wait-setup-ack-ms" => {
                type7_wait_setup_ack_ms =
                    next_value(&mut args, "--type7-wait-setup-ack-ms")?.parse()?
            }
            "--type7-setup-only" => type7_setup_only = true,
            "--dirty-mode" => {
                dirty_mode = parse_dirty_mode(&next_value(&mut args, "--dirty-mode")?)?
            }
            "--dry-run" => dry_run = true,
            "--ram-size-mb" => ram_size_mb = Some(next_value(&mut args, "--ram-size-mb")?.parse()?),
            "--usb-timeout-ms" => {
                usb_timeout_ms = next_value(&mut args, "--usb-timeout-ms")?.parse()?
            }
            "--wait-interrupt-ms" => {
                wait_interrupt_ms = next_value(&mut args, "--wait-interrupt-ms")?.parse()?
            }
            "--dump-interrupts" => {
                dump_interrupts = next_value(&mut args, "--dump-interrupts")?.parse()?
            }
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, "--max-packet")?)?,
            "--dump-first-frame" => {
                dump_first_frame = Some(PathBuf::from(next_value(&mut args, "--dump-first-frame")?))
            }
            "--dump-first-frame-delay-ms" => {
                dump_first_frame_delay_ms =
                    next_value(&mut args, "--dump-first-frame-delay-ms")?.parse()?
            }
            "--dump-type7-packets" => {
                dump_type7_packets = Some(PathBuf::from(next_value(
                    &mut args,
                    "--dump-type7-packets",
                )?))
            }
            "--type7-cmd-addr" => {
                type7_cmd_addr = Some(parse_u32(&next_value(&mut args, "--type7-cmd-addr")?)?)
            }
            "--type7-cmd-wrap-addr" => {
                type7_cmd_wrap_addr =
                    Some(parse_u32(&next_value(&mut args, "--type7-cmd-wrap-addr")?)?)
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
    if !(1..=100).contains(&min_quality) {
        return Err("--min-quality must be 1..100".into());
    }
    if min_quality > quality {
        return Err("--min-quality must be less than or equal to --quality".into());
    }
    if jpeg_encoder == JpegEncoder::T6Yuv420 && subsamp != JpegSubsampling::Yuv420 {
        return Err("--jpeg-encoder t6-yuv420 requires --subsamp 420".into());
    }
    if matches!(
        jpeg_encoder,
        JpegEncoder::VideoToolbox | JpegEncoder::VideoToolboxFullTurboTiles
    ) && !matches!(transport, Transport::Jpeg | Transport::JpegType7)
    {
        return Err(
            "--jpeg-encoder videotoolbox variants require --transport jpeg or jpeg-type7".into(),
        );
    }
    if !(0.0..=1.0).contains(&type7_min_band_ratio) {
        return Err("--type7-min-band-ratio must be 0.0..1.0".into());
    }
    if !(0.0..=1.0).contains(&type7_min_local_area_ratio) {
        return Err("--type7-min-local-area-ratio must be 0.0..1.0".into());
    }
    if !(0.0..=1.0).contains(&type7_max_tile_ratio) {
        return Err("--type7-max-tile-ratio must be 0.0..1.0".into());
    }
    if !(0.0..=1.0).contains(&type7_large_tile_ratio) {
        return Err("--type7-large-tile-ratio must be 0.0..1.0".into());
    }
    if let Some(type7_large_tile_quality) = type7_large_tile_quality {
        if !(1..=100).contains(&type7_large_tile_quality) {
            return Err("--type7-large-tile-quality must be 1..100".into());
        }
    }
    if type7_large_tile_fps_divisor == 0 {
        return Err("--type7-large-tile-fps-divisor must be at least 1".into());
    }
    if let (Some(type7_cmd_addr), Some(type7_cmd_wrap_addr)) = (type7_cmd_addr, type7_cmd_wrap_addr)
    {
        if type7_cmd_wrap_addr <= type7_cmd_addr {
            return Err("--type7-cmd-wrap-addr must be greater than --type7-cmd-addr".into());
        }
    }

    Ok(Options {
        display_index,
        width,
        height,
        rotate,
        fps,
        frames,
        quality,
        adaptive_quality,
        min_quality,
        subsamp,
        jpeg_encoder,
        jpeg_target,
        chroma_mode,
        yuv_matrix,
        yuv_range,
        transport,
        capture_format,
        raw_bulk_mode,
        ready,
        power_on,
        reset_jpeg_engine,
        profile,
        report_every,
        async_send,
        frame_throttle,
        drop_late_frames,
        unsafe_generated_type7,
        type7_rect_mode,
        type7_ring_mode,
        type7_refresh_frames,
        type7_min_band_ratio,
        type7_min_local_area_ratio,
        type7_max_tile_ratio,
        type7_large_tile_ratio,
        type7_large_tile_quality,
        type7_large_tile_fps_divisor,
        type7_setup_mode,
        type7_jpeg_component_ids,
        type7_wait_setup_ack_ms,
        type7_setup_only,
        dirty_mode,
        dry_run,
        ram_size_mb,
        usb_timeout_ms,
        wait_interrupt_ms,
        dump_interrupts,
        max_packet_size,
        dump_first_frame,
        dump_first_frame_delay_ms,
        dump_type7_packets,
        type7_cmd_addr,
        type7_cmd_wrap_addr,
    })
}

fn parse_type7_rect_mode(value: &str) -> Result<Type7RectMode, Box<dyn Error>> {
    match value {
        "auto" => Ok(Type7RectMode::Auto),
        "local" => Ok(Type7RectMode::Local),
        "horizontal-band" | "hband" => Ok(Type7RectMode::HorizontalBand),
        "vertical-band" | "vband" => Ok(Type7RectMode::VerticalBand),
        _ => Err("--type7-rect must be one of auto, local, horizontal-band, vertical-band".into()),
    }
}

fn parse_type7_ring_mode(value: &str) -> Result<Type7RingMode, Box<dyn Error>> {
    match value {
        "windows" | "win" => Ok(Type7RingMode::Windows),
        "legacy" => Ok(Type7RingMode::Legacy),
        _ => Err("--type7-ring must be one of windows, legacy".into()),
    }
}

fn parse_type7_setup_mode(value: &str) -> Result<Type7SetupMode, Box<dyn Error>> {
    match value {
        "legacy-full-jpeg" | "legacy" | "full-jpeg" => Ok(Type7SetupMode::LegacyFullJpeg),
        "synthetic-type4" | "type4" => Ok(Type7SetupMode::SyntheticType4),
        "paired-type4-type7" | "paired" => Ok(Type7SetupMode::PairedType4Type7),
        _ => Err(
            "--type7-setup must be one of legacy-full-jpeg, synthetic-type4, paired-type4-type7"
                .into(),
        ),
    }
}

fn parse_type7_jpeg_component_ids(value: &str) -> Result<Type7JpegComponentIds, Box<dyn Error>> {
    match value {
        "zero-based" | "zero" | "0" => Ok(Type7JpegComponentIds::ZeroBased),
        "original" | "encoder" | "asis" | "as-is" => Ok(Type7JpegComponentIds::Original),
        _ => Err("--type7-jpeg-component-ids must be one of zero-based, original".into()),
    }
}

fn parse_dirty_mode(value: &str) -> Result<DirtyMode, Box<dyn Error>> {
    match value {
        "off" => Ok(DirtyMode::Off),
        "log" => Ok(DirtyMode::Log),
        "bbox" => Ok(DirtyMode::Bbox),
        "tile-send" => Ok(DirtyMode::TileSend),
        _ => Err("--dirty-mode must be one of off, log, bbox, tile-send".into()),
    }
}

fn parse_jpeg_target(value: &str) -> Result<JpegTarget, Box<dyn Error>> {
    match value {
        "nv12" => Ok(JpegTarget::Nv12),
        "yv12" => Ok(JpegTarget::Yv12),
        "yuv444" => Ok(JpegTarget::Yuv444),
        _ => Err("--jpeg-target must be one of nv12, yv12, yuv444".into()),
    }
}

fn parse_jpeg_encoder(value: &str) -> Result<JpegEncoder, Box<dyn Error>> {
    match value {
        "turbo-bgra" | "turbo" => Ok(JpegEncoder::TurboBgra),
        "t6-yuv420" => Ok(JpegEncoder::T6Yuv420),
        "videotoolbox" | "vt" => Ok(JpegEncoder::VideoToolbox),
        "vt-full-turbo-tiles" | "videotoolbox-full-turbo-tiles" | "hybrid-vt" => {
            Ok(JpegEncoder::VideoToolboxFullTurboTiles)
        }
        _ => Err(
            "--jpeg-encoder must be one of turbo-bgra, t6-yuv420, videotoolbox, vt-full-turbo-tiles"
                .into(),
        ),
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
        "jpeg-type7" | "type7" => Ok(Transport::JpegType7),
        "nv12" => Ok(Transport::Nv12),
        "nv12-type7" | "type7-nv12" => Ok(Transport::Nv12Type7),
        "rgb24" => Ok(Transport::Rgb24),
        "yv12" => Ok(Transport::Yv12),
        "yuv444" => Ok(Transport::Yuv444),
        _ => Err(
            "--transport must be one of jpeg, jpeg-type7, nv12, nv12-type7, rgb24, yv12, yuv444"
                .into(),
        ),
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
    --width N               Virtual display width, default 1080\n\
    --height N              Virtual display height, default 1920\n\
    --rotate 0|90|180|270   Rotate output before sending, default 90\n\
                            For portrait-on-landscape output, use --width 1080 --height 1920 --rotate 90\n\
    --fps N                 Virtual display refresh/send cap, default 60\n\
    --frames N              Stop after N sent frames\n\
    --quality N             TurboJPEG quality 1..100, default 95\n\
    --adaptive-quality      Dynamically lower JPEG quality when frame time exceeds the fps budget\n\
    --min-quality N         Minimum adaptive JPEG quality, default 85\n\
    --subsamp 420|422|444   JPEG chroma subsampling, default 420\n\
    --jpeg-encoder turbo-bgra|t6-yuv420|videotoolbox|vt-full-turbo-tiles\n\
                            JPEG encoder input path, default vt-full-turbo-tiles\n\
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
    --transport jpeg|jpeg-type7|nv12|nv12-type7|rgb24|yv12|yuv444\n\
                            Frame transport, default jpeg-type7\n\
    --ready                 Send software-ready before capture\n\
    --power-on              Send monitor power-on before capture\n\
    --reset-jpeg-engine     Send vendor JPEG engine reset before capture\n\
    --profile               Print average convert/encode/packet/USB timings every 60 sent frames\n\
    --report-every N        Print frame status every N sent frames, default 60; 0 disables periodic status\n\
    --async-send            Copy latest captured frame in the callback and send from a worker thread\n\
    --frame-throttle        Drop callbacks before the next frame deadline, default disabled\n\
    --no-frame-throttle     Send every callback frame instead of dropping callbacks before the next frame deadline\n\
    --drop-late-frames      Drop JPEG frames after encode if they already missed the frame budget, default enabled\n\
    --no-drop-late-frames   Send late JPEG frames instead of dropping them\n\
    --unsafe-generated-type7\n\
                            Enable generated type7 dirty JPEG updates, default enabled. Experimental; can corrupt display state\n\
    --no-unsafe-generated-type7\n\
                            Disable generated type7 dirty JPEG updates\n\
    --type7-rect auto|local|horizontal-band|vertical-band\n\
                            Type7 dirty rectangle shaping, default local\n\
    --type7-ring windows|legacy\n\
                            Type7 payload ring allocator, default windows\n\
    --type7-refresh-frames N\n\
                            Send a full JPEG refresh every N Type7-capable frames, default 0; 0 disables periodic refresh\n\
    --type7-min-band-ratio N\n\
                            Fall back to full JPEG when a Type7 band is smaller than this screen ratio, default 0.20\n\
    --type7-min-local-area-ratio N\n\
                            Fall back to full JPEG when a Type7 local tile is smaller than this screen area ratio, default 0\n\
    --type7-max-tile-ratio N\n\
                            Fall back to full JPEG when a Type7 tile is larger than this screen area ratio, default 1.0\n\
    --type7-large-tile-ratio N\n\
                            Tile area ratio threshold for --type7-large-tile-quality, default 0.25\n\
    --type7-large-tile-quality N\n\
                            Override JPEG quality for large Type7 tiles only, default 88\n\
    --type7-large-tile-fps-divisor N\n\
                            Send large Type7 tiles every N frame intervals, default 2\n\
    --type7-setup legacy-full-jpeg|synthetic-type4|paired-type4-type7\n\
                            Full refresh/setup strategy for jpeg-type7, default paired-type4-type7\n\
    --type7-jpeg-component-ids zero-based|original\n\
                            Component IDs in Type7 JPEG SOF/SOS segments, default original\n\
    --type7-wait-setup-ack-ms N\n\
                            In paired Type4+Type7 mode, wait up to N ms for setup ack before Type7, default 0\n\
    --type7-setup-only      In paired Type4+Type7 mode, send only the Type4 setup packet and skip Type7\n\
    --type7-cmd-addr N      Override Type7 command ring start address, default 0x03200000\n\
    --type7-cmd-wrap-addr N Override Type7 command ring wrap address, default 0x03900000\n\
    --dirty-mode off|log|bbox|tile-send\n\
                            Dirty rect profiling mode, default log. bbox/tile-send crop-encode dirty rect tiles for measurement only\n\
    --dry-run               Capture/encode/packetize but do not open USB or send\n\
    --ram-size-mb N         RAM size for dry-run address planning, default 58\n\
    --usb-timeout-ms N      USB transfer timeout, default 3000\n\
    --wait-interrupt-ms N   After each sent frame, read display interrupts for up to N ms, default 0\n\
    --dump-interrupts N     Print the first N raw interrupt packets; use with --wait-interrupt-ms\n\
    --max-packet N          Bulk fragment size, default 0x19000\n\
    --dump-first-frame PATH Save a captured BGRA frame after RGB conversion\n\
    --dump-first-frame-delay-ms N\n\
                            Wait at least N ms before saving --dump-first-frame\n\
    --dump-type7-packets DIR\n\
                            Save generated Type4/Type7 video payloads and bulk metadata for byte comparison"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        ChromaMode, Rotation, Type7JpegComponentIds, YuvMatrix, YuvRange, bgra_to_nv12,
        bgra_to_yv12, patch_jpeg_component_ids_zero_based, patch_jpeg_type7_mode6_compat,
        rgb_to_nv12, rgb_to_yv12, rotate_bgra_with_vimage, source_coordinate,
        type7_mode6_plane_addrs,
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

    #[test]
    fn vimage_bgra_rotation_matches_existing_rgb_rotation() {
        let rgb = sample_rgb_4x2();
        let bgra = bgra_from_rgb(&rgb);

        for rotation in [Rotation::Deg90, Rotation::Deg180, Rotation::Deg270] {
            let mut rotated_bgra = Vec::new();
            let (width, height, stride) =
                rotate_bgra_with_vimage(bgra.as_ptr(), 4, 2, 4 * 4, rotation, &mut rotated_bgra)
                    .unwrap();
            let rotated_rgb = rgb_from_bgra(&rotated_bgra, width, height, stride);

            let mut expected_rgb = Vec::new();
            super::copy_bgra_to_rgb_bytes(bgra.as_ptr(), 4, 2, 4 * 4, rotation, &mut expected_rgb);

            assert_eq!(rotated_rgb, expected_rgb, "rotation {rotation:?}");
        }
    }

    #[test]
    fn type7_mode6_chroma_address_uses_canvas_width_stride() {
        let (plane0, plane1, plane2) =
            type7_mode6_plane_addrs(0x0295_56c0, 0, 128, 1920, 1080, 1920, 1920);

        assert_eq!(plane0, 0x0299_16f0);
        assert_eq!(plane1, 0x02b7_16f0);
        assert_eq!(plane2, 0);
    }

    #[test]
    fn type7_jpeg_component_patch_rewrites_sof_and_sos_ids() {
        let mut jpeg = vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00, 0x10, 0x03, 0x01, 0x22,
            0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xff, 0xda, 0x00, 0x0c, 0x03, 0x01, 0x00,
            0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0xff, 0xd9,
        ];

        patch_jpeg_component_ids_zero_based(&mut jpeg).unwrap();

        assert_eq!(&jpeg[12..21], &[0, 0x22, 0, 1, 0x11, 1, 2, 0x11, 1]);
        assert_eq!(&jpeg[26..32], &[0, 0, 1, 0x11, 2, 0x11]);
    }

    #[test]
    fn type7_jpeg_original_component_ids_preserves_sof_and_sos_ids() {
        let mut jpeg = sample_jpeg_with_one_based_component_ids();

        patch_jpeg_type7_mode6_compat(&mut jpeg, Type7JpegComponentIds::Original).unwrap();

        assert!(
            jpeg.windows(9)
                .any(|window| window == [1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1])
        );
        assert!(
            jpeg.windows(6)
                .any(|window| window == [1, 0, 2, 0x11, 3, 0x11])
        );
    }

    fn sample_jpeg_with_one_based_component_ids() -> Vec<u8> {
        vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00, 0x10, 0x03, 0x01, 0x22,
            0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xff, 0xda, 0x00, 0x0c, 0x03, 0x01, 0x00,
            0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0xff, 0xd9,
        ]
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

    fn rgb_from_bgra(bgra: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            let row = &bgra[y * stride..y * stride + width * 4];
            for pixel in row.chunks_exact(4) {
                rgb.push(pixel[2]);
                rgb.push(pixel[1]);
                rgb.push(pixel[0]);
            }
        }
        rgb
    }
}
