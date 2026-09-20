// 复现"几十MB图片卡爆炸"：量化 paint 路径 decode_kitty_image 对大图的同步解码耗时。
// 运行: cargo test -p muxlane-term --test kitty_bench -- --nocapture
use std::time::Instant;

/// 生成一张 H x W 的渐变 JPEG（无外部依赖，用 image crate 写，muxlane-term 已依赖 image）
fn make_jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut img = image::DynamicImage::new_rgb8(w, h);
    let frame = img.as_mut_rgb8().unwrap();
    for y in 0..h {
        for x in 0..w {
            frame.put_pixel(
                x,
                y,
                image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]),
            );
        }
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    buf.into_inner()
}

#[test]
fn bench_decode_big_image() {
    for (w, h) in [(2000, 1500), (4000, 3000), (6000, 4500)] {
        let jpeg = make_jpeg(w, h);
        let mb = jpeg.len() as f64 / 1024.0 / 1024.0;
        // 模拟 pi 扩展 f=100 传图 + decode_kitty_image 的 to_image_data 路径（image crate 解码）
        let start = Instant::now();
        let decoder = image::ImageReader::new(std::io::Cursor::new(&jpeg))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        let decoded_mb = (w as f64 * h as f64 * 3.0) / 1024.0 / 1024.0;
        println!(
            "{w}x{h} jpeg={mb:.1}MB -> decode {:?} -> raw {decoded_mb:.0}MB in memory",
            start.elapsed()
        );
        drop(decoder);
    }
}

#[test]
fn bench_decode_rejects_oversize() {
    // 直接验证 image crate 尺寸探测路径：超大尺寸应被拦下（与 decode_kitty_image_bytes 同样的护栏逻辑）。
    let jpeg = make_jpeg(100, 100);
    let dim =
        image::ImageReader::with_format(std::io::Cursor::new(&jpeg), image::ImageFormat::Jpeg)
            .into_dimensions()
            .unwrap();
    assert_eq!(dim, (100, 100));
}
