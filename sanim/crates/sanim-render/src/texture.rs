//! Raster baking reuses the ordinary 2D renderer and shared native text/media caches.
use sanim_core::{Animation, Color, Element, Fraction, Viewport};

pub fn install_texture_drawing() {
    sanim_core::math::texture::draw::install(sanim_core::math::texture::draw::Backend {
        rasterize,
        font_epoch: || {
            shrimply_fonts::native::system_font_epoch()
                .map(|epoch| format!("{}:{}", epoch.revision, epoch.serial))
        },
    });
}
fn rasterize(content: &Element, size: [u32; 2], work: usize) -> Result<Vec<u8>, String> {
    let viewport = Viewport {
        width: size[0],
        height: size[1],
        dpi: 96,
        ..Default::default()
    };
    let mut animation = Animation::for_viewport(&viewport);
    animation.background = Color::parse("transparent")?;
    animation.duration = Fraction::from(1);
    animation.add(content.clone(), Fraction::from(0))?;
    let mut renderer = crate::Renderer::default();
    let scene = renderer.prepare(animation)?;
    let mut options = crate::Options::for_animation(scene.animation());
    options.max_pixels = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .ok_or("texture drawing dimensions overflow")?;
    options.max_work = work;
    options.encode_png = false;
    let recording = renderer.record(&scene, Fraction::from(0), &options)?;
    Ok(recording.rasterize(false)?.rgba)
}
