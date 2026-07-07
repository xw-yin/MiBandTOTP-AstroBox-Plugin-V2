use image::{GrayImage, Luma, imageops};
use std::{env, fs};

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: cargo run --example debug_qr -- <image>");
        std::process::exit(2);
    };

    let data = fs::read(&path).expect("read image");
    let image = image::load_from_memory(&data)
        .expect("load image")
        .to_luma8();
    println!("image: {}x{}", image.width(), image.height());
    try_decode("original", image.clone());
    try_decode("threshold-128", threshold(&image, 128));
    try_decode("threshold-180", threshold(&image, 180));
    try_decode(
        "crop-threshold-180",
        crop_and_pad(&threshold(&image, 180), 24),
    );
    try_decode(
        "crop-threshold-180-scale2",
        imageops::resize(
            &crop_and_pad(&threshold(&image, 180), 24),
            crop_and_pad(&threshold(&image, 180), 24).width() * 2,
            crop_and_pad(&threshold(&image, 180), 24).height() * 2,
            imageops::FilterType::Nearest,
        ),
    );
}

fn try_decode(name: &str, image: GrayImage) {
    let mut prepared = rqrr::PreparedImage::prepare(image);
    let grids = prepared.detect_grids();
    println!("{name}: grids={}", grids.len());

    for (index, grid) in grids.into_iter().enumerate() {
        match grid.decode() {
            Ok((meta, content)) => {
                println!("grid #{index}: meta={meta:?}");
                println!("chars: {}", content.chars().count());
                println!("{content}");
            }
            Err(error) => {
                println!("grid #{index}: decode error: {error:?}");
            }
        }
    }
}

fn threshold(image: &GrayImage, level: u8) -> GrayImage {
    let mut out = GrayImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let value = if pixel.0[0] < level { 0 } else { 255 };
        out.put_pixel(x, y, Luma([value]));
    }
    out
}

fn crop_and_pad(image: &GrayImage, quiet: u32) -> GrayImage {
    let (mut min_x, mut min_y) = (image.width(), image.height());
    let (mut max_x, mut max_y) = (0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[0] < 128 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if min_x > max_x || min_y > max_y {
        return image.clone();
    }

    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;
    let cropped = imageops::crop_imm(image, min_x, min_y, width, height).to_image();
    let mut out = GrayImage::from_pixel(width + quiet * 2, height + quiet * 2, Luma([255]));
    imageops::replace(&mut out, &cropped, quiet as i64, quiet as i64);
    out
}
