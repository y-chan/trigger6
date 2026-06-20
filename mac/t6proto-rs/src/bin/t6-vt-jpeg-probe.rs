use std::env;
use std::error::Error;

#[derive(Clone, Copy, Debug)]
struct Options {
    width: usize,
    height: usize,
    quality: f64,
    frames: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            quality: 0.85,
            frames: 60,
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn t6_vt_jpeg_probe(width: usize, height: usize, quality: f64, frames: u32) -> i32;
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    std::hint::black_box(t6proto::Edid::from_blocks(Vec::new()).is_base_checksum_valid());

    #[cfg(target_os = "macos")]
    {
        let status = unsafe {
            t6_vt_jpeg_probe(
                options.width,
                options.height,
                options.quality,
                options.frames,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(format!("VideoToolbox JPEG probe failed with status {status}").into())
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = options;
        Err("t6-vt-jpeg-probe is only supported on macOS".into())
    }
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options::default();
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" => options.width = next_value(&mut args, "--width")?.parse()?,
            "--height" => options.height = next_value(&mut args, "--height")?.parse()?,
            "--quality" => options.quality = next_value(&mut args, "--quality")?.parse()?,
            "--frames" => options.frames = next_value(&mut args, "--frames")?.parse()?,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }

    if options.width == 0 || options.height == 0 {
        return Err("--width and --height must be greater than zero".into());
    }
    if !(0.0..=1.0).contains(&options.quality) {
        return Err("--quality must be 0.0..1.0 for VideoToolbox".into());
    }
    if options.frames == 0 {
        return Err("--frames must be greater than zero".into());
    }

    Ok(options)
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value").into())
}

fn print_help() {
    print!(
        "Usage: t6-vt-jpeg-probe [options]\n\
\n\
Options:\n\
    --width N       Test frame width, default 1920\n\
    --height N      Test frame height, default 1080\n\
    --quality F     VideoToolbox quality 0.0..1.0, default 0.85\n\
    --frames N      Number of frames to encode, default 60\n\
    -h, --help      Show this help\n"
    );
}
