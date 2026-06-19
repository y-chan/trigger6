#![forbid(unsafe_code)]

#[cfg(feature = "usb")]
pub mod usb;

pub const VENDOR_ID: u16 = 0x0711;
pub const PRODUCT_ID_JUA365: u16 = 0x5601;

pub const EP_BULK_OUT: u8 = 0x02;
pub const EP_BULK_IN: u8 = 0x81;
pub const EP_INTERRUPT_IN: u8 = 0x83;

pub const VENDOR_REQ_SET_MONITOR_CONTROL: u8 = 0x03;
pub const VENDOR_REQ_SET_RESOLUTION: u8 = 0x08;
pub const VENDOR_REQ_SET_RESOLUTION_DETAIL_TIMING: u8 = 0x12;
pub const VENDOR_REQ_SET_SOFTWARE_READY: u8 = 0x31;
pub const VENDOR_REQ_GET_EDID: u8 = 0x80;
pub const VENDOR_REQ_GET_RESOLUTION_TABLE_COUNT: u8 = 0x84;
pub const VENDOR_REQ_GET_RESOLUTION_TABLE_DATA: u8 = 0x85;
pub const VENDOR_REQ_RESET_JPEG_ENGINE: u8 = 0x86;
pub const VENDOR_REQ_QUERY_MONITOR_CONNECTION_STATUS: u8 = 0x87;
pub const VENDOR_REQ_QUERY_VIDEO_RAM_SIZE: u8 = 0x88;
pub const VENDOR_REQ_GET_RESOLUTION_TIMING_TABLE: u8 = 0x89;

pub const SIGNATURE_DISPLAY: u32 = 0x00;
pub const SIGNATURE_AUDIO: u32 = 0x03;
pub const SIGNATURE_NV12DMA: u32 = 0x07;

pub const PACKET_FLAG_NONE: u8 = 0x00;
pub const PACKET_FLAG_CONTINUE_TO_SEND: u8 = 0x01;

pub const DISPLAY_EXT_NONE: u8 = 0x00;
pub const DISPLAY_EXT_FLIP_PRIMARY: u8 = 0x03;
pub const DISPLAY_EXT_FLIP_SECONDARY: u8 = 0x04;
pub const DISPLAY_EXT_CLIP_PRIMARY: u8 = 0x05;
pub const DISPLAY_EXT_CLIP_SECONDARY: u8 = 0x06;

pub const INT_FUNC_DISPLAY: u8 = 0x04;
pub const INT_DISPLAY_EVENT_HDMI_CONNECT: u8 = 0x01;
pub const INT_DISPLAY_EVENT_VGA_CONNECT: u8 = 0x02;
pub const INT_DISPLAY_EVENT_FENCE_ID: u8 = 0x04;
pub const INT_DISPLAY_EVENT_JPEG_ERROR: u8 = 0x08;

pub const VIDEO_CMD_CLIP_PRIMARY: u32 = 1;
pub const VIDEO_CMD_CLIP_SECONDARY: u32 = 2;
pub const VIDEO_CMD_FLIP_PRIMARY: u32 = 3;
pub const VIDEO_CMD_FLIP_SECONDARY: u32 = 4;

pub const VIDEO_COLOR_YV12: u32 = 4;
pub const VIDEO_COLOR_NV12: u32 = 6;
pub const VIDEO_COLOR_RGB32: u32 = 8;
pub const VIDEO_COLOR_RGB24: u32 = 9;
pub const VIDEO_COLOR_YUV444: u32 = 11;
pub const VIDEO_COLOR_JPEG: u32 = 13;

pub const BULK_DMA_HEADER_SIZE: usize = 32;
pub const VIDEO_FLIP_HEADER_SIZE: usize = 48;
pub const INTERRUPT_PACKET_SIZE: usize = 64;
pub const EDID_BLOCK_SIZE: usize = 128;
pub const EDID_MAX_BLOCKS: usize = 4;
pub const DEFAULT_MAX_BULK_PACKET_SIZE: u32 = 0x19000;
pub const JPEG_PADDING_SIZE: usize = 1024;
pub const VIDEO_FLAG_RESET_JPEG: u8 = 0x80;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edid {
    blocks: Vec<[u8; EDID_BLOCK_SIZE]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EdidDetailedTiming {
    pub pixel_clock_khz: u32,
    pub horizontal_active: u16,
    pub horizontal_blanking: u16,
    pub vertical_active: u16,
    pub vertical_blanking: u16,
    pub refresh_hz: Option<u16>,
}

impl Edid {
    pub fn from_blocks(blocks: Vec<[u8; EDID_BLOCK_SIZE]>) -> Self {
        Self { blocks }
    }

    pub fn blocks(&self) -> &[[u8; EDID_BLOCK_SIZE]] {
        &self.blocks
    }

    pub fn extension_count(&self) -> u8 {
        self.blocks.first().map(|block| block[126]).unwrap_or(0)
    }

    pub fn declared_block_count(&self) -> usize {
        usize::from(self.extension_count()) + 1
    }

    pub fn clamped_block_count(&self) -> usize {
        self.declared_block_count().min(EDID_MAX_BLOCKS)
    }

    pub fn is_base_checksum_valid(&self) -> bool {
        self.blocks
            .first()
            .map(|block| edid_checksum_is_valid(block))
            .unwrap_or(false)
    }

    pub fn block_checksum_validity(&self) -> Vec<bool> {
        self.blocks
            .iter()
            .map(edid_checksum_is_valid)
            .collect::<Vec<_>>()
    }

    pub fn monitor_name(&self) -> Option<String> {
        self.descriptor_string(0xfc)
    }

    pub fn monitor_serial(&self) -> Option<String> {
        self.descriptor_string(0xff)
    }

    pub fn preferred_timing(&self) -> Option<EdidDetailedTiming> {
        let base = self.blocks.first()?;

        for descriptor_index in 0..4 {
            let start = 54 + descriptor_index * 18;
            let descriptor = &base[start..start + 18];
            let timing = parse_detailed_timing(descriptor);
            if timing.is_some() {
                return timing;
            }
        }

        None
    }

    pub fn has_4k_timing_hint(&self) -> bool {
        self.detailed_timings().iter().any(|timing| {
            timing.horizontal_active >= 3840
                || timing.vertical_active >= 2160
                || (timing.horizontal_active >= 3840 && timing.vertical_active >= 2160)
        })
    }

    pub fn detailed_timings(&self) -> Vec<EdidDetailedTiming> {
        let mut timings = Vec::new();

        if let Some(base) = self.blocks.first() {
            for descriptor_index in 0..4 {
                let start = 54 + descriptor_index * 18;
                if let Some(timing) = parse_detailed_timing(&base[start..start + 18]) {
                    timings.push(timing);
                }
            }
        }

        for extension in self.blocks.iter().skip(1) {
            if extension[0] != 0x02 {
                continue;
            }

            let dtd_start = extension[2] as usize;
            if dtd_start == 0 || dtd_start >= EDID_BLOCK_SIZE {
                continue;
            }

            let mut start = dtd_start;
            while start + 18 <= EDID_BLOCK_SIZE {
                if let Some(timing) = parse_detailed_timing(&extension[start..start + 18]) {
                    timings.push(timing);
                }
                start += 18;
            }
        }

        timings
    }

    fn descriptor_string(&self, tag: u8) -> Option<String> {
        let base = self.blocks.first()?;

        for descriptor_index in 0..4 {
            let start = 54 + descriptor_index * 18;
            let descriptor = &base[start..start + 18];
            if descriptor[0..3] == [0, 0, 0] && descriptor[3] == tag {
                let raw = &descriptor[5..18];
                let text = raw
                    .iter()
                    .copied()
                    .take_while(|byte| *byte != b'\n' && *byte != 0)
                    .collect::<Vec<_>>();
                let value = String::from_utf8_lossy(&text).trim().to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }

        None
    }
}

pub fn edid_checksum_is_valid(block: &[u8; EDID_BLOCK_SIZE]) -> bool {
    block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn parse_detailed_timing(descriptor: &[u8]) -> Option<EdidDetailedTiming> {
    if descriptor.len() != 18 {
        return None;
    }

    let pixel_clock_10khz = u16::from_le_bytes([descriptor[0], descriptor[1]]);
    if pixel_clock_10khz == 0 {
        return None;
    }

    let horizontal_active = u16::from(descriptor[2]) | (u16::from(descriptor[4] & 0xf0) << 4);
    let horizontal_blanking = u16::from(descriptor[3]) | (u16::from(descriptor[4] & 0x0f) << 8);
    let vertical_active = u16::from(descriptor[5]) | (u16::from(descriptor[7] & 0xf0) << 4);
    let vertical_blanking = u16::from(descriptor[6]) | (u16::from(descriptor[7] & 0x0f) << 8);
    let horizontal_total = u32::from(horizontal_active + horizontal_blanking);
    let vertical_total = u32::from(vertical_active + vertical_blanking);
    let pixel_clock_khz = u32::from(pixel_clock_10khz) * 10;
    let refresh_hz = if horizontal_total == 0 || vertical_total == 0 {
        None
    } else {
        Some((pixel_clock_khz * 1000 / horizontal_total / vertical_total) as u16)
    };

    Some(EdidDetailedTiming {
        pixel_clock_khz,
        horizontal_active,
        horizontal_blanking,
        vertical_active,
        vertical_blanking,
        refresh_hz,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BulkDmaHeader {
    pub signature: u32,
    pub payload_length: u32,
    pub payload_address: u32,
    pub packet_size: u32,
    pub packet_offset: u32,
    pub packet_flags: u8,
    pub function_specific: [u8; 8],
}

impl BulkDmaHeader {
    pub fn display(
        payload_length: u32,
        payload_address: u32,
        packet_size: u32,
        packet_offset: u32,
        more_packets: bool,
    ) -> Self {
        Self {
            signature: SIGNATURE_DISPLAY,
            payload_length,
            payload_address,
            packet_size,
            packet_offset,
            packet_flags: if more_packets {
                PACKET_FLAG_CONTINUE_TO_SEND
            } else {
                PACKET_FLAG_NONE
            },
            function_specific: [0; 8],
        }
    }

    pub fn to_bytes(&self) -> [u8; BULK_DMA_HEADER_SIZE] {
        let mut out = [0; BULK_DMA_HEADER_SIZE];
        out[0..4].copy_from_slice(&self.signature.to_le_bytes());
        out[4..8].copy_from_slice(&self.payload_length.to_le_bytes());
        out[8..12].copy_from_slice(&self.payload_address.to_le_bytes());
        out[12..16].copy_from_slice(&self.packet_size.to_le_bytes());
        out[16..20].copy_from_slice(&self.packet_offset.to_le_bytes());
        out[20] = self.packet_flags;
        out[24..32].copy_from_slice(&self.function_specific);
        out
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoFlipHeader {
    pub command: u32,
    pub payload_size: u32,
    pub fence_id: u32,
    pub target_format: u32,
    pub y_rgb_pitch: u16,
    pub uv_pitch: u16,
    pub y_rgb_data_fb_offset: u32,
    pub u_uv_data_offset: u32,
    pub v_data_offset: u32,
    pub source_format: u32,
    pub flags: u8,
}

impl VideoFlipHeader {
    pub fn jpeg(
        display_index: u8,
        payload_size: u32,
        width: u16,
        height: u16,
        framebuffer_address: u32,
        flags: u8,
    ) -> Self {
        Self::jpeg_with_target_format(
            display_index,
            payload_size,
            width,
            height,
            framebuffer_address,
            VIDEO_COLOR_NV12,
            flags,
        )
    }

    pub fn jpeg_with_target_format(
        display_index: u8,
        payload_size: u32,
        width: u16,
        height: u16,
        framebuffer_address: u32,
        target_format: u32,
        flags: u8,
    ) -> Self {
        let pitch = align_u16(width, 32);
        let y_block_size = u32::from(pitch) * u32::from(align_u16(height, 32)) + 1024;

        Self {
            command: flip_command(display_index),
            payload_size,
            fence_id: 0,
            target_format,
            y_rgb_pitch: pitch,
            uv_pitch: pitch,
            y_rgb_data_fb_offset: framebuffer_address,
            u_uv_data_offset: framebuffer_address + y_block_size,
            v_data_offset: 0,
            source_format: VIDEO_COLOR_JPEG,
            flags,
        }
    }

    pub fn nv12(
        display_index: u8,
        payload_size: u32,
        width: u16,
        height: u16,
        framebuffer_address: u32,
        flags: u8,
    ) -> Self {
        let pitch = align_u16(width, 16);
        let uv_offset = u32::from(pitch) * u32::from(height);
        let data_start = framebuffer_address + VIDEO_FLIP_HEADER_SIZE as u32;

        Self {
            command: flip_command(display_index),
            payload_size,
            fence_id: 0,
            target_format: VIDEO_COLOR_NV12,
            y_rgb_pitch: pitch,
            uv_pitch: pitch,
            y_rgb_data_fb_offset: data_start,
            u_uv_data_offset: data_start + uv_offset,
            v_data_offset: 0,
            source_format: VIDEO_COLOR_NV12,
            flags,
        }
    }

    pub fn yv12(
        display_index: u8,
        payload_size: u32,
        width: u16,
        height: u16,
        framebuffer_address: u32,
        flags: u8,
    ) -> Self {
        let y_pitch = align_u16(width, 16);
        let uv_pitch = align_u16(width / 2, 16);
        let y_offset = u32::from(y_pitch) * u32::from(height);
        let v_offset = y_offset + u32::from(uv_pitch) * u32::from(height / 2);
        let data_start = framebuffer_address + VIDEO_FLIP_HEADER_SIZE as u32;

        Self {
            command: flip_command(display_index),
            payload_size,
            fence_id: 0,
            target_format: VIDEO_COLOR_YV12,
            y_rgb_pitch: y_pitch,
            uv_pitch,
            y_rgb_data_fb_offset: data_start,
            u_uv_data_offset: data_start + y_offset,
            v_data_offset: data_start + v_offset,
            source_format: VIDEO_COLOR_YV12,
            flags,
        }
    }

    pub fn rgb24(
        display_index: u8,
        payload_size: u32,
        width: u16,
        framebuffer_address: u32,
        flags: u8,
    ) -> Self {
        let data_start = framebuffer_address + VIDEO_FLIP_HEADER_SIZE as u32;

        Self {
            command: flip_command(display_index),
            payload_size,
            fence_id: 0,
            target_format: VIDEO_COLOR_RGB24,
            y_rgb_pitch: width * 3,
            uv_pitch: 0,
            y_rgb_data_fb_offset: data_start,
            u_uv_data_offset: 0,
            v_data_offset: 0,
            source_format: VIDEO_COLOR_RGB24,
            flags,
        }
    }

    pub fn yuv444(
        display_index: u8,
        payload_size: u32,
        width: u16,
        framebuffer_address: u32,
        flags: u8,
    ) -> Self {
        let data_start = framebuffer_address + VIDEO_FLIP_HEADER_SIZE as u32;

        Self {
            command: flip_command(display_index),
            payload_size,
            fence_id: 0,
            target_format: VIDEO_COLOR_YUV444,
            y_rgb_pitch: width * 3,
            uv_pitch: 0,
            y_rgb_data_fb_offset: data_start,
            u_uv_data_offset: 0,
            v_data_offset: 0,
            source_format: VIDEO_COLOR_YUV444,
            flags,
        }
    }

    pub fn to_bytes(&self) -> [u8; VIDEO_FLIP_HEADER_SIZE] {
        let mut out = [0; VIDEO_FLIP_HEADER_SIZE];
        out[0..4].copy_from_slice(&self.command.to_le_bytes());
        out[4..8].copy_from_slice(&self.payload_size.to_le_bytes());
        out[8..12].copy_from_slice(&self.fence_id.to_le_bytes());
        out[12..16].copy_from_slice(&self.target_format.to_le_bytes());
        out[16..18].copy_from_slice(&self.y_rgb_pitch.to_le_bytes());
        out[18..20].copy_from_slice(&self.uv_pitch.to_le_bytes());
        out[20..24].copy_from_slice(&self.y_rgb_data_fb_offset.to_le_bytes());
        out[24..28].copy_from_slice(&self.u_uv_data_offset.to_le_bytes());
        out[28..32].copy_from_slice(&self.v_data_offset.to_le_bytes());
        out[32..36].copy_from_slice(&self.source_format.to_le_bytes());
        out[47] = self.flags;
        out
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JpegFramePacket {
    pub payload_address: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawFramePacket {
    pub payload_address: u32,
    pub payload: Vec<u8>,
}

impl JpegFramePacket {
    pub fn new(
        display_index: u8,
        jpeg: &[u8],
        width: u16,
        height: u16,
        cmd_addr: u32,
        fb_addr: u32,
        flags: u8,
    ) -> Self {
        Self::new_with_target_format(
            display_index,
            jpeg,
            width,
            height,
            cmd_addr,
            fb_addr,
            VIDEO_COLOR_NV12,
            flags,
        )
    }

    pub fn new_with_target_format(
        display_index: u8,
        jpeg: &[u8],
        width: u16,
        height: u16,
        cmd_addr: u32,
        fb_addr: u32,
        target_format: u32,
        flags: u8,
    ) -> Self {
        let payload_size = (jpeg.len() + JPEG_PADDING_SIZE) as u32;
        let header = VideoFlipHeader::jpeg_with_target_format(
            display_index,
            payload_size,
            width,
            height,
            fb_addr,
            target_format,
            flags,
        );
        let mut payload =
            Vec::with_capacity(VIDEO_FLIP_HEADER_SIZE + jpeg.len() + JPEG_PADDING_SIZE);

        payload.extend_from_slice(&header.to_bytes());
        payload.extend_from_slice(jpeg);
        payload.resize(payload.len() + JPEG_PADDING_SIZE, 0);

        Self {
            payload_address: cmd_addr,
            payload,
        }
    }

    pub fn bulk_chunks(&self, max_packet_size: u32) -> Vec<BulkTransferChunk<'_>> {
        fragments(self.payload.len() as u32, max_packet_size)
            .map(|fragment| BulkTransferChunk {
                header: BulkDmaHeader::display(
                    self.payload.len() as u32,
                    self.payload_address,
                    fragment.size,
                    fragment.offset,
                    fragment.more,
                ),
                data: &self.payload
                    [fragment.offset as usize..(fragment.offset + fragment.size) as usize],
            })
            .collect()
    }
}

impl RawFramePacket {
    pub fn rgb24(
        display_index: u8,
        rgb: &[u8],
        width: u16,
        height: u16,
        fb_addr: u32,
        flags: u8,
    ) -> Self {
        let payload_size = rgb.len() as u32;
        let header = VideoFlipHeader::rgb24(display_index, payload_size, width, fb_addr, flags);
        let mut payload = Vec::with_capacity(VIDEO_FLIP_HEADER_SIZE + rgb.len());

        debug_assert_eq!(rgb.len(), usize::from(width) * usize::from(height) * 3);
        payload.extend_from_slice(&header.to_bytes());
        payload.extend_from_slice(rgb);

        Self {
            payload_address: fb_addr,
            payload,
        }
    }

    pub fn nv12(
        display_index: u8,
        nv12: &[u8],
        width: u16,
        height: u16,
        fb_addr: u32,
        flags: u8,
    ) -> Self {
        let payload_size = (nv12.len() + JPEG_PADDING_SIZE) as u32;
        let header =
            VideoFlipHeader::nv12(display_index, payload_size, width, height, fb_addr, flags);
        let mut payload =
            Vec::with_capacity(VIDEO_FLIP_HEADER_SIZE + nv12.len() + JPEG_PADDING_SIZE);

        payload.extend_from_slice(&header.to_bytes());
        payload.extend_from_slice(nv12);
        payload.resize(payload.len() + JPEG_PADDING_SIZE, 0);

        Self {
            payload_address: fb_addr,
            payload,
        }
    }

    pub fn yv12(
        display_index: u8,
        yv12: &[u8],
        width: u16,
        height: u16,
        fb_addr: u32,
        flags: u8,
    ) -> Self {
        let payload_size = (yv12.len() + JPEG_PADDING_SIZE) as u32;
        let header =
            VideoFlipHeader::yv12(display_index, payload_size, width, height, fb_addr, flags);
        let mut payload =
            Vec::with_capacity(VIDEO_FLIP_HEADER_SIZE + yv12.len() + JPEG_PADDING_SIZE);

        payload.extend_from_slice(&header.to_bytes());
        payload.extend_from_slice(yv12);
        payload.resize(payload.len() + JPEG_PADDING_SIZE, 0);

        Self {
            payload_address: fb_addr,
            payload,
        }
    }

    pub fn yuv444(
        display_index: u8,
        yuv444: &[u8],
        width: u16,
        height: u16,
        fb_addr: u32,
        flags: u8,
    ) -> Self {
        let payload_size = yuv444.len() as u32;
        let header = VideoFlipHeader::yuv444(display_index, payload_size, width, fb_addr, flags);
        let mut payload = Vec::with_capacity(VIDEO_FLIP_HEADER_SIZE + yuv444.len());

        debug_assert_eq!(yuv444.len(), usize::from(width) * usize::from(height) * 3);
        payload.extend_from_slice(&header.to_bytes());
        payload.extend_from_slice(yuv444);

        Self {
            payload_address: fb_addr,
            payload,
        }
    }

    pub fn bulk_chunks(&self, max_packet_size: u32) -> Vec<BulkTransferChunk<'_>> {
        fragments(self.payload.len() as u32, max_packet_size)
            .map(|fragment| BulkTransferChunk {
                header: BulkDmaHeader::display(
                    self.payload.len() as u32,
                    self.payload_address,
                    fragment.size,
                    fragment.offset,
                    fragment.more,
                ),
                data: &self.payload
                    [fragment.offset as usize..(fragment.offset + fragment.size) as usize],
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BulkTransferChunk<'a> {
    pub header: BulkDmaHeader,
    pub data: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VramLayout {
    pub cmd_addr: u32,
    pub fb_addrs: [u32; 3],
}

impl VramLayout {
    pub fn one_port_1080p(ram_size_mb: u8) -> Self {
        let ram_size_mb = u32::from(ram_size_mb);
        Self {
            cmd_addr: 0,
            fb_addrs: [
                (ram_size_mb - 12) * 1024 * 1024,
                (ram_size_mb - 8) * 1024 * 1024,
                (ram_size_mb - 4) * 1024 * 1024,
            ],
        }
    }

    pub fn two_port_1080p_secondary(ram_size_mb: u8) -> Self {
        let ram_size_mb = u32::from(ram_size_mb);
        let cmd_base_mb = ram_size_mb - 18;

        Self {
            cmd_addr: cmd_base_mb * 1024 * 1024,
            fb_addrs: [
                (ram_size_mb - 12) * 1024 * 1024,
                (ram_size_mb - 8) * 1024 * 1024,
                (ram_size_mb - 4) * 1024 * 1024,
            ],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameScheduler {
    initial_cmd_addr: u32,
    cmd_addr: u32,
    fb_addrs: [u32; 3],
    fb_index: usize,
    reset_frames_remaining: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAddresses {
    pub cmd_addr: u32,
    pub fb_addr: u32,
    pub reset_jpeg: bool,
    pub cmd_offset: u32,
}

impl FrameScheduler {
    pub fn new(layout: VramLayout) -> Self {
        Self {
            initial_cmd_addr: layout.cmd_addr,
            cmd_addr: layout.cmd_addr,
            fb_addrs: layout.fb_addrs,
            fb_index: 1,
            reset_frames_remaining: 10,
        }
    }

    pub fn next_jpeg_frame(&mut self, jpeg_len: usize) -> FrameAddresses {
        self.fb_index = (self.fb_index + 1) % self.fb_addrs.len();
        let fb_addr = self.fb_addrs[self.fb_index];
        let cmd_offset = jpeg_cmd_offset(jpeg_len + JPEG_PADDING_SIZE + VIDEO_FLIP_HEADER_SIZE);
        let mut reset_jpeg = false;

        if self.cmd_addr + cmd_offset > self.fb_addrs[0] {
            self.cmd_addr = self.initial_cmd_addr;
            reset_jpeg = true;
        }

        if self.reset_frames_remaining > 0 {
            self.reset_frames_remaining -= 1;
            reset_jpeg = true;
        }

        let addresses = FrameAddresses {
            cmd_addr: self.cmd_addr,
            fb_addr,
            reset_jpeg,
            cmd_offset,
        };
        self.cmd_addr += cmd_offset;

        addresses
    }
}

pub fn jpeg_cmd_offset(payload_len: usize) -> u32 {
    if payload_len < 0x100000 {
        0x100000
    } else if payload_len < 0x200000 {
        0x200000
    } else {
        0x300000
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayInterrupt {
    pub is_display: bool,
    pub display_data: u32,
    pub display_event: u8,
    pub has_fence_id: bool,
    pub has_jpeg_error: bool,
}

impl DisplayInterrupt {
    pub fn parse(packet: &[u8; INTERRUPT_PACKET_SIZE]) -> Self {
        let is_display = packet[0] & INT_FUNC_DISPLAY != 0;
        let display_data = u32::from_le_bytes(packet[12..16].try_into().unwrap());
        let display_event = packet[19];

        Self {
            is_display,
            display_data,
            display_event,
            has_fence_id: is_display && display_event & INT_DISPLAY_EVENT_FENCE_ID != 0,
            has_jpeg_error: is_display && display_event & INT_DISPLAY_EVENT_JPEG_ERROR != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fragment {
    pub offset: u32,
    pub size: u32,
    pub more: bool,
}

pub fn fragments(payload_length: u32, max_packet_size: u32) -> impl Iterator<Item = Fragment> {
    let count = fragment_count(payload_length, max_packet_size);

    (0..count).map(move |index| {
        let offset = fragment_offset(max_packet_size, index);
        let size = fragment_size(payload_length, max_packet_size, index);
        Fragment {
            offset,
            size,
            more: offset + size < payload_length,
        }
    })
}

pub fn fragment_count(payload_length: u32, max_packet_size: u32) -> u32 {
    if payload_length == 0 || max_packet_size == 0 {
        return 0;
    }
    payload_length.div_ceil(max_packet_size)
}

pub fn fragment_size(payload_length: u32, max_packet_size: u32, fragment_index: u32) -> u32 {
    let offset = fragment_offset(max_packet_size, fragment_index);
    if max_packet_size == 0 || offset >= payload_length {
        return 0;
    }

    (payload_length - offset).min(max_packet_size)
}

pub fn fragment_offset(max_packet_size: u32, fragment_index: u32) -> u32 {
    max_packet_size * fragment_index
}

fn flip_command(display_index: u8) -> u32 {
    if display_index == 0 {
        VIDEO_CMD_FLIP_PRIMARY
    } else {
        VIDEO_CMD_FLIP_SECONDARY
    }
}

fn align_u16(value: u16, alignment: u16) -> u16 {
    let value = u32::from(value);
    let alignment = u32::from(alignment);
    value.div_ceil(alignment).checked_mul(alignment).unwrap() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes.try_into().unwrap())
    }

    #[test]
    fn bulk_header_matches_capture_shape() {
        let header = BulkDmaHeader::display(0x87e0, 0x03000000, 0x87e0, 0, false).to_bytes();

        assert_eq!(le32(&header[0..4]), 0);
        assert_eq!(le32(&header[4..8]), 0x87e0);
        assert_eq!(le32(&header[8..12]), 0x03000000);
        assert_eq!(le32(&header[12..16]), 0x87e0);
        assert_eq!(le32(&header[16..20]), 0);
        assert_eq!(header[20], 0);
    }

    #[test]
    fn fragmentation_sets_continue_flag() {
        let total = 0x4f7e0;
        let max_packet = 0x19000;
        let fragments = fragments(total, max_packet).collect::<Vec<_>>();

        assert_eq!(
            fragments,
            vec![
                Fragment {
                    offset: 0,
                    size: 0x19000,
                    more: true
                },
                Fragment {
                    offset: 0x19000,
                    size: 0x19000,
                    more: true
                },
                Fragment {
                    offset: 0x32000,
                    size: 0x19000,
                    more: true
                },
                Fragment {
                    offset: 0x4b000,
                    size: 0x47e0,
                    more: false
                },
            ]
        );

        let header = BulkDmaHeader::display(
            total,
            0x030b7dc0,
            fragments[1].size,
            fragments[1].offset,
            fragments[1].more,
        )
        .to_bytes();

        assert_eq!(le32(&header[12..16]), 0x19000);
        assert_eq!(le32(&header[16..20]), 0x19000);
        assert_eq!(header[20], PACKET_FLAG_CONTINUE_TO_SEND);
    }

    #[test]
    fn jpeg_flip_header_matches_expected_fields() {
        let header = VideoFlipHeader::jpeg(
            0,
            0x87e0 - VIDEO_FLIP_HEADER_SIZE as u32,
            1360,
            768,
            0x50055005,
            0,
        )
        .to_bytes();

        assert_eq!(le32(&header[0..4]), VIDEO_CMD_FLIP_PRIMARY);
        assert_eq!(le32(&header[12..16]), VIDEO_COLOR_NV12);
        assert_eq!(le32(&header[20..24]), 0x50055005);
        assert_eq!(le32(&header[32..36]), VIDEO_COLOR_JPEG);
        assert_eq!(header[47], 0);
    }

    #[test]
    fn interrupt_parsing() {
        let mut packet = [0; INTERRUPT_PACKET_SIZE];
        packet[0] = INT_FUNC_DISPLAY;
        packet[12] = 0x3b;
        packet[19] = INT_DISPLAY_EVENT_FENCE_ID;

        let interrupt = DisplayInterrupt::parse(&packet);

        assert!(interrupt.is_display);
        assert_eq!(interrupt.display_data, 0x3b);
        assert_eq!(interrupt.display_event, INT_DISPLAY_EVENT_FENCE_ID);
        assert!(interrupt.has_fence_id);
        assert!(!interrupt.has_jpeg_error);
    }

    #[test]
    fn edid_counts_extensions_and_checksums() {
        let mut base = [0; EDID_BLOCK_SIZE];
        base[0..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        base[54..72].copy_from_slice(&[
            0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0x00, 0x0f, 0x28,
            0x21, 0x00, 0x00, 0x1e,
        ]);
        base[90..108].copy_from_slice(&[
            0x00, 0x00, 0x00, 0xfc, 0x00, b'V', b'A', b'2', b'4', b'D', b'\n', b' ', b' ', b' ',
            b' ', b' ', b' ', b' ',
        ]);
        base[126] = 1;
        base[127] = base
            .iter()
            .take(127)
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg();

        let mut extension = [0; EDID_BLOCK_SIZE];
        extension[0] = 0x02;
        extension[127] = 0xfe;

        let edid = Edid::from_blocks(vec![base, extension]);

        assert_eq!(edid.extension_count(), 1);
        assert_eq!(edid.declared_block_count(), 2);
        assert_eq!(edid.clamped_block_count(), 2);
        assert_eq!(edid.block_checksum_validity(), vec![true, true]);
        assert!(edid.is_base_checksum_valid());
        assert_eq!(edid.monitor_name().as_deref(), Some("VA24D"));

        let timing = edid.preferred_timing().unwrap();
        assert_eq!(timing.horizontal_active, 1920);
        assert_eq!(timing.vertical_active, 1080);
        assert_eq!(timing.refresh_hz, Some(60));
    }
}
