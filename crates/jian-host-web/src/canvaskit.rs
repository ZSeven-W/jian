//! Narrow wasm-bindgen boundary for the CanvasKit JavaScript bridge.

use js_sys::{Array, Promise};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen(module = "/src/ck_bridge.js")]
extern "C" {
    pub type CkRuntime;
    pub type CkSurface;
    pub type CkImage;

    #[wasm_bindgen(catch, js_name = jianCkInit)]
    fn jian_ck_init(asset_base: &str) -> Result<Promise, JsValue>;

    #[wasm_bindgen(method, catch, js_name = makeSurface)]
    pub fn make_surface(
        this: &CkRuntime,
        canvas: &HtmlCanvasElement,
        width: u32,
        height: u32,
    ) -> Result<CkSurface, JsValue>;
    #[wasm_bindgen(method, catch, js_name = decodeImage)]
    pub fn decode_image(this: &CkRuntime, bytes: &[u8]) -> Result<CkImage, JsValue>;
    #[wasm_bindgen(method, js_name = deleteImage)]
    pub fn delete_image(this: &CkRuntime, image: &CkImage);
    #[wasm_bindgen(method, js_name = dispose)]
    pub fn dispose_runtime(this: &CkRuntime);
    #[wasm_bindgen(method, js_name = registerFont)]
    pub fn register_font(this: &CkRuntime, alias: &str, actual_family: &str, bytes: &[u8]) -> bool;
    #[cfg(all(test, target_arch = "wasm32"))]
    #[wasm_bindgen(method, js_name = registeredFontCount)]
    pub fn registered_font_count(this: &CkRuntime) -> u32;
    #[wasm_bindgen(method, js_name = coversText)]
    pub fn covers_text(this: &CkRuntime, alias: &str, text: &str) -> bool;
    #[wasm_bindgen(method, js_name = measureParagraph)]
    pub fn measure_paragraph(
        this: &CkRuntime,
        texts: &Array,
        families: &Array,
        sizes: &[f32],
        weights: &[u16],
        italics: &[u8],
        letter_spacing: &[f32],
        max_width: f32,
        line_height: f32,
    ) -> Vec<f32>;

    #[wasm_bindgen(method, js_name = beginFrame)]
    pub fn begin_frame(this: &CkSurface, clear: u32, dpr: f32);
    #[wasm_bindgen(method, js_name = endFrame)]
    pub fn end_frame(this: &CkSurface);
    #[wasm_bindgen(method, js_name = pushClip)]
    pub fn push_clip(this: &CkSurface, x: f32, y: f32, width: f32, height: f32);
    #[wasm_bindgen(method, js_name = pushTransform)]
    pub fn push_transform(this: &CkSurface, matrix: &[f32]);
    #[wasm_bindgen(method)]
    pub fn pop(this: &CkSurface);
    #[wasm_bindgen(method, js_name = pushLayer)]
    pub fn push_layer(this: &CkSurface, bounds: &[f32], filter_kind: u8, filter: &[f32]);
    #[wasm_bindgen(method, js_name = drawRect)]
    pub fn draw_rect(
        this: &CkSurface,
        rect: &[f32],
        fill: f64,
        stroke: f64,
        width: f32,
        opacity: f32,
    );
    #[wasm_bindgen(method, js_name = drawRoundedRect)]
    pub fn draw_rounded_rect(
        this: &CkSurface,
        rect: &[f32],
        radii: &[f32],
        fill: f64,
        stroke: f64,
        width: f32,
        opacity: f32,
    );
    #[wasm_bindgen(method, js_name = drawPath)]
    pub fn draw_path(
        this: &CkSurface,
        commands: &[f32],
        fill: f64,
        stroke: f64,
        width: f32,
        opacity: f32,
    );
    #[wasm_bindgen(method, js_name = drawImage)]
    pub fn draw_image(this: &CkSurface, image: &CkImage, rect: &[f32], opacity: f32);
    #[wasm_bindgen(method, js_name = drawText)]
    pub fn draw_text(
        this: &CkSurface,
        text: &str,
        family: &str,
        rect: &[f32],
        size: f32,
        weight: u16,
        color: u32,
        align: u8,
        line_height: f32,
    );
    #[wasm_bindgen(method, js_name = drawRichText)]
    pub fn draw_rich_text(
        this: &CkSurface,
        texts: &Array,
        families: &Array,
        sizes: &[f32],
        weights: &[u16],
        italics: &[u8],
        letter_spacing: &[f32],
        colors: &[u32],
        rect: &[f32],
        align: u8,
        line_height: f32,
    );
    #[wasm_bindgen(method, js_name = drawLinearGradient)]
    pub fn draw_linear_gradient(
        this: &CkSurface,
        rect: &[f32],
        radii: &[f32],
        angle: f32,
        stops: &[f32],
        opacity: f32,
        stroke: f64,
        stroke_width: f32,
    );
    #[wasm_bindgen(method, js_name = drawRadialGradient)]
    pub fn draw_radial_gradient(
        this: &CkSurface,
        rect: &[f32],
        radii: &[f32],
        cx: f32,
        cy: f32,
        radius: f32,
        stops: &[f32],
        opacity: f32,
        stroke: f64,
        stroke_width: f32,
    );
    #[wasm_bindgen(method, js_name = drawMeshGradient)]
    pub fn draw_mesh_gradient(
        this: &CkSurface,
        rect: &[f32],
        radii: &[f32],
        rows: u32,
        cols: u32,
        colors: &[u32],
        opacity: f32,
        stroke: f64,
        stroke_width: f32,
    );
    #[wasm_bindgen(method, js_name = drawShader)]
    pub fn draw_shader(
        this: &CkSurface,
        rect: &[f32],
        radii: &[f32],
        sksl: &str,
        uniform_names: &Array,
        uniforms: &[f32],
        uniform_arities: &[u32],
        opacity: f32,
        fallback: u32,
        stroke: f64,
        stroke_width: f32,
    );
    #[wasm_bindgen(method, js_name = drawShadow)]
    pub fn draw_shadow(this: &CkSurface, rect: &[f32], radii: &[f32], shadow: &[f32]);
    #[wasm_bindgen(method, js_name = drawIcon)]
    pub fn draw_icon(this: &CkSurface, rect: &[f32], name: &str, family: &str, color: u32);
    #[wasm_bindgen(method, js_name = readPixel)]
    pub fn read_pixel(this: &CkSurface, x: u32, y: u32) -> Vec<u8>;
    #[wasm_bindgen(method, js_name = regionHasInk)]
    pub fn region_has_ink(this: &CkSurface, x: u32, y: u32, width: u32, height: u32) -> bool;
    #[wasm_bindgen(method, js_name = lastTextWidth)]
    pub fn last_text_width(this: &CkSurface) -> f32;
    #[wasm_bindgen(method)]
    pub fn dispose(this: &CkSurface);
}

pub fn clone_runtime(runtime: &CkRuntime) -> CkRuntime {
    let value: &JsValue = runtime.as_ref();
    value.clone().unchecked_into()
}

pub async fn load(asset_base: &str) -> Result<CkRuntime, JsValue> {
    let value = wasm_bindgen_futures::JsFuture::from(jian_ck_init(asset_base)?).await?;
    Ok(value.unchecked_into())
}
