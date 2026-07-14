use crate::CanvasKitBackend;
use jian_core::geometry::{rect, size};
use jian_core::render::{
    BorderRadii, DrawOp, GradientStop, ImageSource, LinearGradient, Paint, RenderBackend,
    ShadowSpec,
};
use jian_core::scene::Color;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

fn append_png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut crc_data = Vec::with_capacity(kind.len() + data.len());
    crc_data.extend_from_slice(kind);
    crc_data.extend_from_slice(data);
    png.extend_from_slice(&crc32(&crc_data).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn rgba_png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut scanlines = Vec::with_capacity((height * (1 + width * 4)) as usize);
    for y in 0..height {
        scanlines.push(0);
        for x in 0..width {
            scanlines.extend_from_slice(&pixel(x, y));
        }
    }
    assert!(scanlines.len() <= u16::MAX as usize);

    let length = scanlines.len() as u16;
    let mut zlib = vec![0x78, 0x01, 0x01];
    zlib.extend_from_slice(&length.to_le_bytes());
    zlib.extend_from_slice(&(!length).to_le_bytes());
    zlib.extend_from_slice(&scanlines);
    zlib.extend_from_slice(&adler32(&scanlines).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    append_png_chunk(&mut png, b"IHDR", &ihdr);
    append_png_chunk(&mut png, b"IDAT", &zlib);
    append_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn assert_green(pixel: [u8; 4], label: &str) {
    assert!(
        pixel[0] < 20 && pixel[1] > 235 && pixel[2] < 20 && pixel[3] > 245,
        "{label}: expected opaque green, got {pixel:?}",
    );
}

fn assert_red(pixel: [u8; 4], label: &str) {
    assert!(
        pixel[0] > 235 && pixel[1] < 20 && pixel[2] < 20 && pixel[3] > 245,
        "{label}: expected opaque red, got {pixel:?}",
    );
}

fn assert_blue(pixel: [u8; 4], label: &str) {
    assert!(
        pixel[0] < 20 && pixel[1] < 20 && pixel[2] > 235 && pixel[3] > 245,
        "{label}: expected opaque blue, got {pixel:?}",
    );
}

#[wasm_bindgen_test(async)]
async fn transparent_surface_pixels_do_not_count_as_ink() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let surface = backend.new_surface(size(16.0, 16.0));

    assert!(
        !surface.region_has_ink(0, 0, 16, 16),
        "a not-yet-presented transparent surface must not satisfy pixel-ink probes"
    );
}

#[wasm_bindgen_test(async)]
async fn images_center_crop_to_cover_in_both_aspect_directions() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let wide = rgba_png(12, 4, |x, _| {
        if x < 4 {
            [255, 0, 0, 255]
        } else if x < 8 {
            [0, 255, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let tall = rgba_png(4, 12, |_, y| {
        if y < 4 {
            [255, 0, 0, 255]
        } else if y < 8 {
            [0, 255, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    backend.register_image("fixture:cover-wide", &wide).unwrap();
    backend.register_image("fixture:cover-tall", &tall).unwrap();

    let mut surface = backend.new_surface(size(64.0, 24.0));
    backend.begin_frame(&mut surface, 0x00000000);
    backend.draw(&DrawOp::Image {
        source: ImageSource::Url("fixture:cover-wide".into()),
        dst: rect(0.0, 0.0, 24.0, 24.0),
        opacity: 1.0,
    });
    backend.draw(&DrawOp::Image {
        source: ImageSource::Url("fixture:cover-tall".into()),
        dst: rect(32.0, 0.0, 24.0, 24.0),
        opacity: 1.0,
    });
    backend.end_frame(&mut surface);

    assert_green(surface.read_pixel(4, 12), "wide crop left");
    assert_green(surface.read_pixel(19, 12), "wide crop right");
    assert_green(surface.read_pixel(44, 4), "tall crop top");
    assert_green(surface.read_pixel(44, 19), "tall crop bottom");
}

#[wasm_bindgen_test(async)]
async fn linear_gradient_uses_native_projected_endpoints() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let mut surface = backend.new_surface(size(120.0, 40.0));
    backend.begin_frame(&mut surface, 0x00000000);
    backend.draw(&DrawOp::LinearGradientRect {
        rect: rect(0.0, 0.0, 120.0, 40.0),
        radii: BorderRadii::zero(),
        gradient: LinearGradient {
            angle_deg: 45.0,
            stops: vec![
                GradientStop {
                    offset: -0.5,
                    color: Color::rgb(255, 0, 0),
                },
                GradientStop {
                    offset: 0.1,
                    color: Color::rgb(255, 0, 0),
                },
                GradientStop {
                    offset: 0.2,
                    color: Color::rgb(0, 255, 0),
                },
                GradientStop {
                    offset: 0.8,
                    color: Color::rgb(0, 255, 0),
                },
                GradientStop {
                    offset: 0.9,
                    color: Color::rgb(0, 0, 255),
                },
                GradientStop {
                    offset: 1.5,
                    color: Color::rgb(0, 0, 255),
                },
            ],
            opacity: 1.0,
        },
        stroke: None,
    });
    backend.end_frame(&mut surface);

    assert_green(surface.read_pixel(8, 32), "projected quarter point");
    assert_green(surface.read_pixel(112, 8), "projected three-quarter point");
    assert_red(surface.read_pixel(4, 4), "gradient start direction");
    assert_blue(surface.read_pixel(116, 36), "gradient end direction");
}

#[wasm_bindgen_test(async)]
async fn direct_shadow_spread_outsets_and_insets_parallel_round_rects() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let mut surface = backend.new_surface(size(112.0, 64.0));
    backend.begin_frame(&mut surface, 0x00000000);
    backend.draw(&DrawOp::ShadowedRect {
        rect: rect(24.0, 24.0, 24.0, 24.0),
        radii: BorderRadii::uniform(8.0),
        shadow: ShadowSpec {
            color: Color::rgb(255, 0, 0),
            dx: 0.0,
            dy: 0.0,
            blur: 0.0,
            spread: 4.0,
        },
    });
    backend.draw(&DrawOp::ShadowedRect {
        rect: rect(80.0, 24.0, 24.0, 24.0),
        radii: BorderRadii::uniform(8.0),
        shadow: ShadowSpec {
            color: Color::rgb(0, 0, 255),
            dx: 0.0,
            dy: 0.0,
            blur: 0.0,
            spread: -4.0,
        },
    });
    backend.end_frame(&mut surface);

    let outset = surface.read_pixel(21, 34);
    assert!(
        outset[0] > 235 && outset[3] > 245,
        "positive spread must outset with radius 12, got {outset:?}",
    );
    assert!(
        surface.read_pixel(18, 34)[3] < 10,
        "positive spread must stop outside its expanded edge",
    );
    assert!(
        surface.read_pixel(82, 36)[3] < 10,
        "negative spread must inset the original edge",
    );
    let inset = surface.read_pixel(85, 30);
    assert!(
        inset[2] > 235 && inset[3] > 245,
        "negative spread must shrink radius 8 to 4, got {inset:?}",
    );
}

#[wasm_bindgen_test(async)]
async fn direct_shadow_blur_radius_is_half_sigma() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let mut surface = backend.new_surface(size(160.0, 96.0));
    backend.begin_frame(&mut surface, 0x00000000);
    backend.draw(&DrawOp::ShadowedRect {
        rect: rect(24.0, 24.0, 24.0, 48.0),
        radii: BorderRadii::zero(),
        shadow: ShadowSpec {
            color: Color::rgb(0, 0, 0),
            dx: 0.0,
            dy: 0.0,
            blur: 8.0,
            spread: 0.0,
        },
    });
    backend.apply_blur(4.0);
    backend.push_layer(rect(80.0, 0.0, 80.0, 96.0));
    backend.draw(&DrawOp::Rect {
        rect: rect(104.0, 24.0, 24.0, 48.0),
        paint: Paint::solid(Color::rgb(0, 0, 0)),
    });
    backend.pop_layer();
    backend.end_frame(&mut surface);

    let mut saw_nontrivial_reference = false;
    for distance in [3_u32, 6, 9] {
        let direct = surface.read_pixel(24 - distance, 48)[3];
        let reference = surface.read_pixel(104 - distance, 48)[3];
        saw_nontrivial_reference |= (10..245).contains(&reference);
        assert!(
            direct.abs_diff(reference) <= 12,
            "distance {distance}: direct blur alpha {direct} differs from sigma-4 reference {reference}",
        );
    }
    assert!(
        saw_nontrivial_reference,
        "blur probes must exercise the falloff"
    );
}
