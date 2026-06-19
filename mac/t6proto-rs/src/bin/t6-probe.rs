use std::env;
use std::error::Error;

use t6proto::usb::{T6Device, list_t6_devices};
use t6proto::{EDID_BLOCK_SIZE, PRODUCT_ID_JUA365, VENDOR_ID};

#[derive(Clone, Copy, Debug, Default)]
struct Options {
    claim: bool,
    display_index: u16,
    ready: bool,
    power_on: bool,
    read_edid: bool,
    read_interrupt: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;

    let devices = list_t6_devices()?;
    if devices.is_empty() {
        println!("No MCT Trigger 6 devices found.");
    } else {
        println!("MCT devices:");
        for device in &devices {
            let marker = if device.vendor_id == VENDOR_ID && device.product_id == PRODUCT_ID_JUA365
            {
                " target"
            } else {
                ""
            };
            println!(
                "  bus={} address={} vid={:04x} pid={:04x} class={:02x}/{:02x}/{:02x}{}",
                device.bus_number,
                device.address,
                device.vendor_id,
                device.product_id,
                device.class_code,
                device.sub_class_code,
                device.protocol_code,
                marker
            );
        }
    }

    if !options.claim {
        println!("Use --claim to open and claim the first 0711:5601 device.");
        return Ok(());
    }

    let device = T6Device::open_first()?;
    println!("Claimed interface {}", device.interface_number());

    if options.ready {
        device.send_software_ready(options.display_index)?;
        println!(
            "Sent display software-ready for display {}.",
            options.display_index
        );
    }

    if options.power_on {
        device.set_monitor_power(options.display_index, true)?;
        println!(
            "Sent monitor power on for display {}.",
            options.display_index
        );
    }

    match device.read_video_ram_size_mb() {
        Ok(size) => println!("Video RAM size: {size} MB"),
        Err(error) => println!("Video RAM size read failed: {error}"),
    }

    for display_index in [0, 1] {
        match device.read_monitor_status(display_index) {
            Ok(status) => println!("Display {display_index} monitor status: {status}"),
            Err(error) => println!("Display {display_index} monitor status failed: {error}"),
        }
    }

    if options.read_edid {
        match device.read_edid(options.display_index) {
            Ok(edid) => {
                println!(
                    "Display {} EDID: declared_blocks={} read_blocks={} base_checksum={} name={} serial={} preferred={}",
                    options.display_index,
                    edid.declared_block_count(),
                    edid.blocks().len(),
                    edid.is_base_checksum_valid(),
                    edid.monitor_name().unwrap_or_else(|| "-".to_string()),
                    edid.monitor_serial().unwrap_or_else(|| "-".to_string()),
                    format_timing(edid.preferred_timing())
                );
                println!(
                    "Display {} EDID 4K hint: {}",
                    options.display_index,
                    edid.has_4k_timing_hint()
                );

                for (index, block) in edid.blocks().iter().enumerate() {
                    let offset = index * EDID_BLOCK_SIZE;
                    println!(
                        "Display {} EDID block {} offset {} checksum={}:",
                        options.display_index,
                        index,
                        offset,
                        edid.block_checksum_validity()[index]
                    );
                    print_hex(block);
                }
            }
            Err(error) => println!(
                "Display {} EDID read failed: {error}",
                options.display_index
            ),
        }
    }

    if options.read_interrupt {
        match device.read_interrupt_once() {
            Ok(interrupt) => println!(
                "Interrupt: display={} data=0x{:08x} event=0x{:02x} fence={} jpeg_error={}",
                interrupt.is_display,
                interrupt.display_data,
                interrupt.display_event,
                interrupt.has_fence_id,
                interrupt.has_jpeg_error
            ),
            Err(error) => println!("Interrupt read failed: {error}"),
        }
    }

    Ok(())
}

fn format_timing(timing: Option<t6proto::EdidDetailedTiming>) -> String {
    match timing {
        Some(timing) => format!(
            "{}x{}@{}Hz",
            timing.horizontal_active,
            timing.vertical_active,
            timing
                .refresh_hz
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string())
        ),
        None => "-".to_string(),
    }
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options::default();
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claim" => options.claim = true,
            "--display" => {
                let value = args
                    .next()
                    .ok_or("--display requires a display index, usually 0 or 1")?;
                options.display_index = value.parse()?;
            }
            "--ready" => options.ready = true,
            "--power-on" => options.power_on = true,
            "--edid" => options.read_edid = true,
            "--interrupt" => options.read_interrupt = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }

    Ok(options)
}

fn print_help() {
    println!(
        "Usage: t6-probe [--claim] [--display 0|1] [--ready] [--power-on] [--edid] [--interrupt]\n\
\n\
Default mode only lists matching MCT devices. Use --claim before any USB\n\
control or interrupt operations. If the vendor driver owns the device, --claim\n\
will likely fail until that driver is stopped or uninstalled."
    );
}

fn print_hex(bytes: &[u8]) {
    for (index, chunk) in bytes.chunks(16).enumerate() {
        print!("  {:04x}:", index * 16);
        for byte in chunk {
            print!(" {byte:02x}");
        }
        println!();
    }
}
