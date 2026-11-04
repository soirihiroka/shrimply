#[cfg(target_os = "linux")]
use shrimply_preview_runtime_cuda::gl_loader;
#[cfg(target_os = "linux")]
#[path = "../src/intel_gl.rs"]
mod intel_gl;

#[cfg(target_os = "linux")]
fn main() -> Result<(), String> {
    use glow::HasContext;
    use std::ffi::c_void;

    // Use Mesa's native surfaceless EGL platform, so this probe needs no desktop.
    const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31dd;
    const EGL_OPENGL_API: u32 = 0x30a2;
    const EGL_NONE: i32 = 0x3038;
    const EGL_SURFACE_TYPE: i32 = 0x3033;
    const EGL_PBUFFER_BIT: i32 = 1;
    const EGL_RENDERABLE_TYPE: i32 = 0x3040;
    const EGL_OPENGL_BIT: i32 = 8;
    const EGL_WIDTH: i32 = 0x3057;
    const EGL_HEIGHT: i32 = 0x3056;
    const EGL_CONTEXT_MAJOR_VERSION: i32 = 0x3098;
    const EGL_CONTEXT_MINOR_VERSION: i32 = 0x30fb;
    const SIZE: u32 = 16;
    let egl = unsafe { libloading::Library::new("libEGL.so.1") }.map_err(|e| e.to_string())?;
    macro_rules! function {
        ($name:literal, $type:ty) => {
            unsafe {
                *egl.get::<$type>(concat!($name, "\0").as_bytes())
                    .map_err(|e| e.to_string())?
            }
        };
    }
    let display = function!(
        "eglGetPlatformDisplay",
        unsafe extern "C" fn(u32, *mut c_void, *const isize) -> *mut c_void
    );
    let initialize = function!(
        "eglInitialize",
        unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32) -> u32
    );
    let bind_api = function!("eglBindAPI", unsafe extern "C" fn(u32) -> u32);
    let choose = function!(
        "eglChooseConfig",
        unsafe extern "C" fn(*mut c_void, *const i32, *mut *mut c_void, i32, *mut i32) -> u32
    );
    let pbuffer = function!(
        "eglCreatePbufferSurface",
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const i32) -> *mut c_void
    );
    let context = function!(
        "eglCreateContext",
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const i32) -> *mut c_void
    );
    let current = function!(
        "eglMakeCurrent",
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> u32
    );
    let destroy_context = function!(
        "eglDestroyContext",
        unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32
    );
    let destroy_surface = function!(
        "eglDestroySurface",
        unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32
    );
    let terminate = function!("eglTerminate", unsafe extern "C" fn(*mut c_void) -> u32);
    unsafe {
        let display = display(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if display.is_null()
            || initialize(display, std::ptr::null_mut(), std::ptr::null_mut()) == 0
            || bind_api(EGL_OPENGL_API) == 0
        {
            return Err("Initialize surfaceless EGL OpenGL".into());
        }
        let mut config = std::ptr::null_mut();
        let mut count = 0;
        if choose(
            display,
            [
                EGL_SURFACE_TYPE,
                EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_BIT,
                EGL_NONE,
            ]
            .as_ptr(),
            &mut config,
            1,
            &mut count,
        ) == 0
            || count == 0
        {
            return Err("Select EGL OpenGL configuration".into());
        }
        let surface = pbuffer(
            display,
            config,
            [EGL_WIDTH, SIZE as i32, EGL_HEIGHT, SIZE as i32, EGL_NONE].as_ptr(),
        );
        if surface.is_null() {
            return Err("Create EGL pbuffer".into());
        }
        let started = std::time::Instant::now();
        let mut renderer = shrimply_render_vulkan::Renderer::new()?;
        println!("Intel VA-API/Vulkan initialized in {:?}", started.elapsed());
        let frame = renderer.composite_buffers(&[], SIZE, SIZE, 0xff00_00ff)?;
        println!("Slang composite submitted after {:?}", started.elapsed());
        let (vector, vector_ready) = renderer.draw_vector((SIZE, SIZE), |canvas| {
            canvas.clear(skia_safe::Color::RED);
            Ok(())
        })?;
        vector_ready.wait()?;
        println!("Skia Vulkan render completed after {:?}", started.elapsed());
        // Exercise both argument blocks that exceed Vulkan's portable push limit.
        let red = [1.0_f32, 0.0, 0.0, 1.0].map(f32::to_ne_bytes);
        let count = SIZE as usize * SIZE as usize;
        let background = renderer.allocate(count * size_of::<u32>())?;
        let mut arguments = renderer.arguments("render_background")?;
        arguments
            .set("output", &background.address().to_ne_bytes())?
            .set("uniforms.common.width", &SIZE.to_ne_bytes())?
            .set("uniforms.common.height", &SIZE.to_ne_bytes())?
            .set("uniforms.gradient.scale", &1.0_f32.to_ne_bytes())?
            .set("uniforms.gradient.color_a", red.as_flattened())?
            .set("uniforms.gradient.color_b", red.as_flattened())?;
        let background_ready = renderer.dispatch(
            arguments,
            vec![background.clone()],
            [SIZE as usize, SIZE as usize, 1],
        )?;
        background_ready.wait()?;
        let convolution = renderer.allocate(count * size_of::<[f32; 4]>())?;
        let mut arguments = renderer.arguments("convolution")?;
        arguments
            .set("params.width", &SIZE.to_ne_bytes())?
            .set("params.height", &SIZE.to_ne_bytes())?
            .set("params.bias", red.as_flattened())?
            .set("params.result_scale", &1.0_f32.to_ne_bytes())?
            .set("output", &convolution.address().to_ne_bytes())?
            .set("output_count", &(count as u64).to_ne_bytes())?;
        let convolution_ready =
            renderer.dispatch(arguments, vec![convolution.clone()], [count, 1, 1])?;
        convolution_ready.wait()?;
        if renderer
            .readback(&convolution)?
            .chunks_exact(size_of::<[f32; 4]>())
            .any(|pixel| pixel != red.as_flattened())
        {
            return Err("Intel convolution argument-buffer mismatch".into());
        }
        for (source, buffer) in [
            ("Slang", &frame.output),
            ("Skia", &vector),
            ("Background", &background),
        ] {
            let (texture, ready) = renderer.presentation_texture(buffer, SIZE, SIZE)?;
            ready.wait()?;
            // Import the same immutable image into two independent GL contexts.
            for index in 0..2 {
                let context = context(
                    display,
                    config,
                    std::ptr::null_mut(),
                    [
                        EGL_CONTEXT_MAJOR_VERSION,
                        3,
                        EGL_CONTEXT_MINOR_VERSION,
                        3,
                        EGL_NONE,
                    ]
                    .as_ptr(),
                );
                if context.is_null() || current(display, surface, surface, context) == 0 {
                    return Err("Make EGL OpenGL context current".into());
                }
                let gl = glow::Context::from_loader_function(gl_loader::proc_address);
                println!(
                    "{source} GL context {index}: {}",
                    gl.get_parameter_string(glow::RENDERER)
                );
                let imported = intel_gl::ImportedTexture::new(&gl, &texture)?;
                let framebuffer = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(imported.texture),
                    0,
                );
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    return Err("Imported Intel texture framebuffer is incomplete".into());
                }
                let mut pixel = [0_u8; 4];
                gl.read_pixels(
                    0,
                    0,
                    1,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixel)),
                );
                if pixel != [255, 0, 0, 255] || gl.get_error() != glow::NO_ERROR {
                    return Err(format!("Intel Vulkan/GL pixel mismatch: {pixel:?}"));
                }
                gl.delete_framebuffer(framebuffer);
                imported.destroy(&gl);
                current(
                    display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                destroy_context(display, context);
            }
        }
        destroy_surface(display, surface);
        terminate(display);
    }
    println!("Intel Vulkan/OpenGL texture and semaphore imports passed");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("The Intel GL probe requires Linux");
}
