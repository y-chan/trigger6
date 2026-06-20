use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use t6proto::usb::T6Device;
use t6proto::{BulkDmaHeader, fragments};

#[derive(Clone, Debug)]
struct Options {
    manifest_json: PathBuf,
    record: Option<usize>,
    record_start: Option<usize>,
    record_end: Option<usize>,
    video_type: Option<u32>,
    sequence_start: Option<u32>,
    payload_addr: Option<u32>,
    ready: bool,
    power_on: bool,
    dry_run: bool,
    max_packet_size: u32,
    wait_interrupt_ms: u64,
    dump_interrupts: u32,
    sleep_ms: u64,
    dump_header: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    replay_manifest(&options)
}

fn replay_manifest(options: &Options) -> Result<(), Box<dyn Error>> {
    if options.dry_run {
        println!("Dry run; replay payloads will not be sent.");
    }

    let json = fs::read_to_string(&options.manifest_json)?;
    let manifest: ReplayManifest = serde_json::from_str(&json)?;
    if manifest.records.is_empty() {
        return Err("replay manifest contains no records".into());
    }

    let base_dir = options
        .manifest_json
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let selected = manifest
        .records
        .iter()
        .filter(|record| {
            options.record.is_none_or(|index| record.index == index)
                && options
                    .record_start
                    .is_none_or(|start| record.index >= start)
                && options.record_end.is_none_or(|end| record.index <= end)
                && options
                    .video_type
                    .is_none_or(|video_type| record.video.video_type == video_type)
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err("selected replay range contains no records".into());
    }

    println!(
        "Replaying video manifest records={} selected={} pcap={}",
        manifest.count,
        selected.len(),
        manifest.pcap
    );

    let device = if options.dry_run {
        None
    } else {
        Some(T6Device::open_first().map_err(|error| format!("open device failed: {error}"))?)
    };
    if let Some(device) = &device {
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
    }

    let mut sequence = options.sequence_start;
    let mut payload_addr = options.payload_addr;

    for (selected_index, record) in selected.iter().enumerate() {
        let payload_path = base_dir.join(&record.files.payload);
        let mut payload = fs::read(&payload_path)?;
        let cmd_dest = payload_addr.unwrap_or(record.command.dest);
        let fence_id = sequence.unwrap_or(record.video.sequence);
        if sequence.is_some() {
            patch_payload_sequence(&mut payload, fence_id)?;
        }
        if payload.len() != record.command.total_len as usize {
            println!(
                "warning: record {} payload_len={} command.total_len={}",
                record.index,
                payload.len(),
                record.command.total_len
            );
        }

        let chunk_count = t6proto::fragment_count(payload.len() as u32, options.max_packet_size);
        println!(
            "replay record {}/{} index={} type={} seq=0x{:08x} cmd_dest=0x{:08x} payload_bytes={} chunks={} size={}x{} jpeg={}x{} start=0x{:08x} end=0x{:08x} components={}",
            selected_index + 1,
            selected.len(),
            record.index,
            record.video.video_type,
            fence_id,
            cmd_dest,
            payload.len(),
            chunk_count,
            record.video.width_field,
            record.video.height_field,
            record.video.jpeg_width.unwrap_or(0),
            record.video.jpeg_height.unwrap_or(0),
            record.video.start_addr,
            record.video.end_addr,
            record.video.jpeg_components.as_deref().unwrap_or("?"),
        );
        if options.dump_header {
            let header_len = 48.min(payload.len());
            println!("payload_header={}", hex_bytes(&payload[..header_len]));
        }

        if let Some(device) = &device {
            send_raw_payload(device, options, cmd_dest, fence_id, &payload)?;
        }

        if let Some(current) = sequence {
            sequence = Some(current.wrapping_add(1));
        }
        if let Some(current) = payload_addr {
            payload_addr = Some(next_ring_addr(current, payload.len()));
        }

        if options.sleep_ms > 0 && selected_index + 1 < selected.len() {
            thread::sleep(Duration::from_millis(options.sleep_ms));
        }
    }

    Ok(())
}

fn patch_payload_sequence(payload: &mut [u8], sequence: u32) -> Result<(), Box<dyn Error>> {
    if payload.len() < 12 {
        return Err("payload is too small to patch sequence".into());
    }
    payload[8..12].copy_from_slice(&sequence.to_le_bytes());
    Ok(())
}

fn send_raw_payload(
    device: &T6Device,
    options: &Options,
    payload_addr: u32,
    sequence: u32,
    payload: &[u8],
) -> Result<(), Box<dyn Error>> {
    let payload_len = payload.len() as u32;
    let chunks = fragments(payload_len, options.max_packet_size)
        .map(|fragment| {
            let data =
                &payload[fragment.offset as usize..(fragment.offset + fragment.size) as usize];
            (
                BulkDmaHeader::display(
                    payload_len,
                    payload_addr,
                    fragment.size,
                    fragment.offset,
                    fragment.more,
                ),
                data,
            )
        })
        .collect::<Vec<_>>();

    for (index, (header, data)) in chunks.iter().enumerate() {
        if let Err(error) = device.write_display_bulk(&header.to_bytes()) {
            if options.wait_interrupt_ms > 0 {
                let _ = wait_for_interrupts(
                    device,
                    sequence,
                    Duration::from_millis(options.wait_interrupt_ms),
                    options.dump_interrupts,
                );
            }
            return Err(format!(
                "replay bulk header error at chunk {}/{} offset={} size={}: {error}",
                index + 1,
                chunks.len(),
                header.packet_offset,
                header.packet_size
            )
            .into());
        }
        if let Err(error) = device.write_display_bulk(data) {
            if options.wait_interrupt_ms > 0 {
                let _ = wait_for_interrupts(
                    device,
                    sequence,
                    Duration::from_millis(options.wait_interrupt_ms),
                    options.dump_interrupts,
                );
            }
            return Err(format!(
                "replay bulk data error at chunk {}/{} offset={} size={}: {error}",
                index + 1,
                chunks.len(),
                header.packet_offset,
                header.packet_size
            )
            .into());
        }
        println!(
            "sent replay chunk {}/{} offset={} size={}",
            index + 1,
            chunks.len(),
            header.packet_offset,
            header.packet_size
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

#[derive(Debug, Deserialize)]
struct ReplayManifest {
    pcap: String,
    count: usize,
    records: Vec<ReplayRecord>,
}

#[derive(Debug, Deserialize)]
struct ReplayRecord {
    index: usize,
    command: ReplayCommand,
    video: ReplayVideo,
    files: ReplayFiles,
}

#[derive(Debug, Deserialize)]
struct ReplayCommand {
    total_len: u32,
    dest: u32,
}

#[derive(Debug, Deserialize)]
struct ReplayVideo {
    #[serde(rename = "type")]
    video_type: u32,
    sequence: u32,
    width_field: u16,
    height_field: u16,
    start_addr: u32,
    end_addr: u32,
    jpeg_width: Option<u16>,
    jpeg_height: Option<u16>,
    jpeg_components: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReplayFiles {
    payload: String,
}

fn next_ring_addr(payload_addr: u32, payload_len: usize) -> u32 {
    let aligned = ((payload_len as u32).saturating_add(0x3ff)) & !0x3ff;
    payload_addr.wrapping_add(aligned.saturating_sub(32))
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
    let mut manifest_json = None;
    let mut record = None;
    let mut record_start = None;
    let mut record_end = None;
    let mut video_type = None;
    let mut sequence_start = None;
    let mut payload_addr = None;
    let mut ready = false;
    let mut power_on = false;
    let mut dry_run = false;
    let mut max_packet_size = 0x8000;
    let mut wait_interrupt_ms = 0;
    let mut dump_interrupts = 0;
    let mut sleep_ms = 0;
    let mut dump_header = false;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" | "--replay-manifest-json" => {
                manifest_json = Some(PathBuf::from(next_value(&mut args, &arg)?))
            }
            "--record" | "--replay-record" => record = Some(next_value(&mut args, &arg)?.parse()?),
            "--record-start" | "--replay-record-start" => {
                record_start = Some(next_value(&mut args, &arg)?.parse()?)
            }
            "--record-end" | "--replay-record-end" => {
                record_end = Some(next_value(&mut args, &arg)?.parse()?)
            }
            "--type" | "--replay-type" => {
                video_type = Some(parse_u32(&next_value(&mut args, &arg)?)?)
            }
            "--sequence-start" | "--replay-sequence-start" => {
                sequence_start = Some(parse_u32(&next_value(&mut args, &arg)?)?)
            }
            "--payload-addr" | "--replay-payload-addr" => {
                payload_addr = Some(parse_u32(&next_value(&mut args, &arg)?)?)
            }
            "--max-packet" => max_packet_size = parse_u32(&next_value(&mut args, &arg)?)?,
            "--wait-interrupt-ms" => wait_interrupt_ms = next_value(&mut args, &arg)?.parse()?,
            "--dump-interrupts" => dump_interrupts = next_value(&mut args, &arg)?.parse()?,
            "--sleep-ms" | "--scan-sleep-ms" => sleep_ms = next_value(&mut args, &arg)?.parse()?,
            "--dump-header" => dump_header = true,
            "--ready" => ready = true,
            "--power-on" => power_on = true,
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }

    Ok(Options {
        manifest_json: manifest_json.ok_or("--manifest is required")?,
        record,
        record_start,
        record_end,
        video_type,
        sequence_start,
        payload_addr,
        ready,
        power_on,
        dry_run,
        max_packet_size,
        wait_interrupt_ms,
        dump_interrupts,
        sleep_ms,
        dump_header,
    })
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value").into())
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
        "Usage: t6-replay-video --manifest manifest.json [options]\n\
Options:\n\
    --manifest PATH          Replay manifest from tools/t6_reassemble_video.py --export-payloads\n\
    --record N              Optional manifest record index to replay\n\
    --record-start N        Optional first manifest record index to replay\n\
    --record-end N          Optional last manifest record index to replay\n\
    --type N                Optional video type filter, e.g. 4 or 7\n\
    --sequence-start N      Rewrite payload sequence/fence ids starting at N\n\
    --payload-addr N        Rewrite bulk payload address and advance as a ring\n\
    --max-packet N          Bulk fragment size, default 0x8000\n\
    --wait-interrupt-ms N   Read interrupts after each record for up to N ms\n\
    --dump-interrupts N     Print first N raw interrupt packets for each record\n\
    --sleep-ms N            Sleep between records, default 0\n\
    --dump-header           Print the first 48 payload header bytes\n\
    --ready                 Send software-ready before replay\n\
    --power-on              Send monitor power-on before replay\n\
    --dry-run               Parse and print selected records but do not open USB or send"
    );
}
