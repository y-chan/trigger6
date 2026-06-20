use std::time::Duration;

use rusb::{Context, Device, DeviceDescriptor, DeviceHandle, Direction, Recipient, request_type};
use rusb::{RequestType, UsbContext};

use crate::{
    DisplayInterrupt, EDID_BLOCK_SIZE, EDID_MAX_BLOCKS, EP_BULK_OUT, EP_INTERRUPT_IN, Edid,
    INTERRUPT_PACKET_SIZE, JpegFramePacket, PRODUCT_ID_JUA365, RawFramePacket, VENDOR_ID,
    VENDOR_REQ_GET_EDID, VENDOR_REQ_QUERY_MONITOR_CONNECTION_STATUS,
    VENDOR_REQ_QUERY_VIDEO_RAM_SIZE, VENDOR_REQ_RESET_JPEG_ENGINE, VENDOR_REQ_SET_MONITOR_CONTROL,
    VENDOR_REQ_SET_SOFTWARE_READY,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct T6DeviceInfo {
    pub bus_number: u8,
    pub address: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub class_code: u8,
    pub sub_class_code: u8,
    pub protocol_code: u8,
}

pub struct T6Device {
    handle: DeviceHandle<Context>,
    interface_number: u8,
    timeout: Duration,
}

impl T6DeviceInfo {
    fn from_device(device: &Device<Context>, descriptor: &DeviceDescriptor) -> Self {
        Self {
            bus_number: device.bus_number(),
            address: device.address(),
            vendor_id: descriptor.vendor_id(),
            product_id: descriptor.product_id(),
            class_code: descriptor.class_code(),
            sub_class_code: descriptor.sub_class_code(),
            protocol_code: descriptor.protocol_code(),
        }
    }
}

impl T6Device {
    pub fn open_first() -> rusb::Result<Self> {
        let context = Context::new()?;
        let devices = context.devices()?;

        for device in devices.iter() {
            let descriptor = device.device_descriptor()?;
            if descriptor.vendor_id() == VENDOR_ID && descriptor.product_id() == PRODUCT_ID_JUA365 {
                return Self::open_device(device, &descriptor);
            }
        }

        Err(rusb::Error::NoDevice)
    }

    pub fn open_device(
        device: Device<Context>,
        descriptor: &DeviceDescriptor,
    ) -> rusb::Result<Self> {
        let interface_number = preferred_interface_number(&device, descriptor).unwrap_or(0);
        let handle = device.open()?;
        handle.claim_interface(interface_number)?;

        Ok(Self {
            handle,
            interface_number,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn interface_number(&self) -> u8 {
        self.interface_number
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn read_video_ram_size_mb(&self) -> rusb::Result<u8> {
        let mut buf = [0; 1];
        self.read_vendor_control(VENDOR_REQ_QUERY_VIDEO_RAM_SIZE, 0, 0, &mut buf)?;
        Ok(buf[0])
    }

    pub fn read_monitor_status(&self, display_index: u16) -> rusb::Result<u8> {
        let mut buf = [0; 1];
        self.read_vendor_control(
            VENDOR_REQ_QUERY_MONITOR_CONNECTION_STATUS,
            display_index,
            0,
            &mut buf,
        )?;
        Ok(buf[0])
    }

    pub fn read_edid_block(&self, display_index: u16, offset: u16) -> rusb::Result<[u8; 128]> {
        let mut buf = [0; EDID_BLOCK_SIZE];
        self.read_vendor_control(VENDOR_REQ_GET_EDID, offset, display_index, &mut buf)?;
        Ok(buf)
    }

    pub fn read_edid(&self, display_index: u16) -> rusb::Result<Edid> {
        let base = self.read_edid_block(display_index, 0)?;
        let block_count = (usize::from(base[126]) + 1).min(EDID_MAX_BLOCKS);
        let mut blocks = Vec::with_capacity(block_count);

        blocks.push(base);
        for block_index in 1..block_count {
            blocks
                .push(self.read_edid_block(display_index, (block_index * EDID_BLOCK_SIZE) as u16)?);
        }

        Ok(Edid::from_blocks(blocks))
    }

    pub fn send_software_ready(&self, display_index: u16) -> rusb::Result<()> {
        self.write_vendor_control(VENDOR_REQ_SET_SOFTWARE_READY, display_index, 0, &[])
            .map(|_| ())
    }

    pub fn set_monitor_power(&self, display_index: u16, enabled: bool) -> rusb::Result<()> {
        self.write_vendor_control(
            VENDOR_REQ_SET_MONITOR_CONTROL,
            display_index,
            u16::from(enabled),
            &[],
        )
        .map(|_| ())
    }

    pub fn reset_jpeg_engine(&self, display_index: u16) -> rusb::Result<()> {
        self.write_vendor_control(VENDOR_REQ_RESET_JPEG_ENGINE, display_index, 0, &[])
            .map(|_| ())
    }

    pub fn read_interrupt_once(&self) -> rusb::Result<DisplayInterrupt> {
        self.read_interrupt_once_timeout(self.timeout)
    }

    pub fn read_interrupt_once_timeout(&self, timeout: Duration) -> rusb::Result<DisplayInterrupt> {
        let mut packet = [0; INTERRUPT_PACKET_SIZE];
        self.handle
            .read_interrupt(EP_INTERRUPT_IN, &mut packet, timeout)?;
        Ok(DisplayInterrupt::parse(&packet))
    }

    pub fn write_display_bulk(&self, data: &[u8]) -> rusb::Result<usize> {
        self.handle.write_bulk(EP_BULK_OUT, data, self.timeout)
    }

    pub fn send_jpeg_frame(
        &self,
        packet: &JpegFramePacket,
        max_packet_size: u32,
    ) -> rusb::Result<()> {
        for chunk in packet.bulk_chunks(max_packet_size) {
            self.write_display_bulk(&chunk.header.to_bytes())?;
            self.write_display_bulk(chunk.data)?;
        }

        Ok(())
    }

    pub fn send_raw_frame(
        &self,
        packet: &RawFramePacket,
        max_packet_size: u32,
    ) -> rusb::Result<()> {
        for chunk in packet.bulk_chunks(max_packet_size) {
            self.write_display_bulk(&chunk.header.to_bytes())?;
            self.write_display_bulk(chunk.data)?;
        }

        Ok(())
    }

    fn read_vendor_control(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &mut [u8],
    ) -> rusb::Result<usize> {
        self.handle.read_control(
            request_type(Direction::In, RequestType::Vendor, Recipient::Device),
            request,
            value,
            index,
            data,
            self.timeout,
        )
    }

    fn write_vendor_control(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> rusb::Result<usize> {
        self.handle.write_control(
            request_type(Direction::Out, RequestType::Vendor, Recipient::Device),
            request,
            value,
            index,
            data,
            self.timeout,
        )
    }
}

pub fn list_t6_devices() -> rusb::Result<Vec<T6DeviceInfo>> {
    let context = Context::new()?;
    let devices = context.devices()?;
    let mut infos = Vec::new();

    for device in devices.iter() {
        let descriptor = device.device_descriptor()?;
        if descriptor.vendor_id() == VENDOR_ID {
            infos.push(T6DeviceInfo::from_device(&device, &descriptor));
        }
    }

    Ok(infos)
}

fn preferred_interface_number(
    device: &Device<Context>,
    _descriptor: &DeviceDescriptor,
) -> Option<u8> {
    let config = device.active_config_descriptor().ok()?;

    for interface in config.interfaces() {
        for descriptor in interface.descriptors() {
            let has_bulk_out = descriptor
                .endpoint_descriptors()
                .any(|endpoint| endpoint.address() == EP_BULK_OUT);
            if has_bulk_out {
                return Some(descriptor.interface_number());
            }
        }
    }

    config
        .interfaces()
        .next()
        .and_then(|interface| interface.descriptors().next())
        .map(|descriptor| descriptor.interface_number())
}
