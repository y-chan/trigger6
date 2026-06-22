use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use image::imageops::FilterType;
use turbojpeg::{Compressor, OutputBuf, Subsamp, YuvPlanesImage};

#[derive(Clone, Debug)]
struct Options {
    input: PathBuf,
    output: PathBuf,
    width: Option<u16>,
    height: Option<u16>,
    quality: i32,
    subsamp: Subsamp,
    encoder: Encoder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Encoder {
    TurboRgb,
    T6Yuv420,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let image = image::open(&options.input)?;
    let rgb = match (options.width, options.height) {
        (Some(width), Some(height)) => image
            .resize_exact(u32::from(width), u32::from(height), FilterType::Lanczos3)
            .to_rgb8(),
        (None, None) => image.to_rgb8(),
        _ => return Err("--width and --height must be specified together".into()),
    };

    let jpeg = match options.encoder {
        Encoder::TurboRgb => {
            turbojpeg::compress_image(&rgb, options.quality, options.subsamp)?.to_vec()
        }
        Encoder::T6Yuv420 => {
            if options.subsamp != Subsamp::Sub2x2 {
                return Err("--encoder t6-yuv420 requires --subsamp 420".into());
            }
            encode_t6_yuv420(
                rgb.as_raw(),
                rgb.width() as usize,
                rgb.height() as usize,
                options.quality,
            )?
        }
    };
    fs::write(&options.output, &jpeg)?;
    println!(
        "wrote {} encoder={:?} quality={} subsamp={:?} bytes={}",
        options.output.display(),
        options.encoder,
        options.quality,
        options.subsamp,
        jpeg.len()
    );
    Ok(())
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut input = None;
    let mut output = None;
    let mut width = None;
    let mut height = None;
    let mut quality = 95;
    let mut subsamp = Subsamp::Sub2x2;
    let mut encoder = Encoder::TurboRgb;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = Some(PathBuf::from(next_value(&mut args, "--input")?)),
            "--output" => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--width" => width = Some(next_value(&mut args, "--width")?.parse()?),
            "--height" => height = Some(next_value(&mut args, "--height")?.parse()?),
            "--quality" => quality = next_value(&mut args, "--quality")?.parse()?,
            "--subsamp" => subsamp = parse_subsamp(&next_value(&mut args, "--subsamp")?)?,
            "--encoder" => encoder = parse_encoder(&next_value(&mut args, "--encoder")?)?,
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
        input: input.ok_or("--input is required")?,
        output: output.ok_or("--output is required")?,
        width,
        height,
        quality,
        subsamp,
        encoder,
    })
}

fn encode_t6_yuv420(
    rgb: &[u8],
    width: usize,
    height: usize,
    quality: i32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if width % 2 != 0 || height % 2 != 0 {
        return Err("t6-yuv420 requires even width and height".into());
    }
    let mut y_plane = vec![0u8; width * height];
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let mut u_plane = vec![0u8; chroma_width * chroma_height];
    let mut v_plane = vec![0u8; chroma_width * chroma_height];

    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 3;
            let r = rgb[i] as f32;
            let g = rgb[i + 1] as f32;
            let b = rgb[i + 2] as f32;
            y_plane[y * width + x] = clamp_u8((0.299 * r) + (0.587 * g) + (0.114 * b));
        }
    }

    for cy in 0..chroma_height {
        for cx in 0..chroma_width {
            let mut r_sum = 0.0f32;
            let mut g_sum = 0.0f32;
            let mut b_sum = 0.0f32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let x = cx * 2 + dx;
                    let y = cy * 2 + dy;
                    let i = (y * width + x) * 3;
                    r_sum += rgb[i] as f32;
                    g_sum += rgb[i + 1] as f32;
                    b_sum += rgb[i + 2] as f32;
                }
            }
            let r = r_sum * 0.25;
            let g = g_sum * 0.25;
            let b = b_sum * 0.25;
            let u = (-0.168736 * r) - (0.331264 * g) + (0.5 * b) + 128.0;
            let v = (0.5 * r) - (0.418688 * g) - (0.081312 * b) + 128.0;
            let ci = cy * chroma_width + cx;
            u_plane[ci] = clamp_u8(u);
            v_plane[ci] = clamp_u8(v);
        }
    }

    let image = YuvPlanesImage {
        y_plane: y_plane.as_slice(),
        u_plane: u_plane.as_slice(),
        v_plane: v_plane.as_slice(),
        width,
        height,
        y_stride: width,
        u_stride: chroma_width,
        v_stride: chroma_width,
        subsamp: Subsamp::Sub2x2,
    };
    let mut compressor = Compressor::new()?;
    compressor.set_quality(quality)?;
    let mut output = OutputBuf::new_owned();
    compressor.compress_yuv_planes(&image, &mut output)?;
    Ok(output.into_owned().to_vec())
}

fn clamp_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn parse_encoder(value: &str) -> Result<Encoder, Box<dyn Error>> {
    match value {
        "turbo-rgb" => Ok(Encoder::TurboRgb),
        "t6-yuv420" => Ok(Encoder::T6Yuv420),
        _ => Err("--encoder must be one of turbo-rgb, t6-yuv420".into()),
    }
}

fn parse_subsamp(value: &str) -> Result<Subsamp, Box<dyn Error>> {
    match value {
        "420" | "4:2:0" => Ok(Subsamp::Sub2x2),
        "422" | "4:2:2" => Ok(Subsamp::Sub2x1),
        "444" | "4:4:4" => Ok(Subsamp::None),
        _ => Err("--subsamp must be one of 420, 422, 444".into()),
    }
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
        "Usage: t6-encode-jpeg --input PATH --output PATH [options]\n\
\n\
Options:\n\
    --width N --height N      Resize before encoding\n\
    --quality N              TurboJPEG quality 1..100, default 95\n\
    --subsamp 420|422|444    Chroma subsampling, default 420\n\
    --encoder turbo-rgb|t6-yuv420\n\
                              Encoding path, default turbo-rgb\n"
    );
}
