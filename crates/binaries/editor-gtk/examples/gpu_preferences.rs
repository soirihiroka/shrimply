#[cfg(target_os = "linux")]
use adw::prelude::*;
#[cfg(target_os = "linux")]
use shrimply_preferences_core as preferences;
#[cfg(target_os = "linux")]
use shrimply_visual_cuda::gpu;
#[cfg(target_os = "linux")]
use std::{cell::RefCell, rc::Rc, time::Duration};

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use shrimply_project_document::project::Time;
    use shrimply_visual_cuda::compositor::{CompositeAccuracy, VideoCommand, VideoEvent};

    if let Some(expected) = std::env::args().nth(1) {
        gpu::configure_gtk_backend()?;
        gpu::configure_gtk_presentation()?;
        let selected =
            preferences::snapshot(&preferences::open_with_defaults()).acceleration_device;
        if selected != expected
            || (!selected.is_empty()
                && gpu::intel_backend_selected() != selected.starts_with("intel:"))
        {
            return Err(
                "Saved acceleration device selected the wrong backend after restart".into(),
            );
        }
        gpu::preflight()?;
        let (commands, events) = preview_worker();
        gtk::init()?;
        let area = gtk::GLArea::builder()
            .allowed_apis(gtk::gdk::GLAPI::GL)
            .width_request(64)
            .height_request(64)
            .build();
        area.set_required_version(3, 3);
        let window = gtk::Window::builder().child(&area).build();
        window.present();
        let context = gtk::glib::MainContext::default();
        while !area.is_realized() {
            context.iteration(true);
        }
        area.make_current();
        if let Some(error) = area.error() {
            return Err(error.into());
        }
        let address = shrimply_components_gtk::gl_loader::proc_address("glGetString");
        if address.is_null() {
            return Err("GTK OpenGL context lacks glGetString".into());
        }
        let get_string: unsafe extern "system" fn(u32) -> *const std::ffi::c_char =
            unsafe { std::mem::transmute(address) };
        // GL_RENDERER, reported by the actual GTK GLArea context.
        let renderer = unsafe { get_string(0x1f01) };
        if renderer.is_null() {
            return Err("GTK OpenGL context has no renderer".into());
        }
        println!("GTK renderer: {}", unsafe {
            std::ffi::CStr::from_ptr(renderer).to_string_lossy()
        });
        let renderer = Rc::new(RefCell::new(
            shrimply_preview_runtime_cuda::renderer::VideoRenderer::new()?,
        ));
        let current_frame = Rc::new(RefCell::new(None));
        let result = Rc::new(RefCell::new(None));
        let rendered = result.clone();
        let frame_to_present = current_frame.clone();
        let presenter = renderer.clone();
        area.connect_render(move |area, _| {
            let frame = frame_to_present.borrow();
            if let Some((frame, expected)) = frame.as_ref() {
                *rendered.borrow_mut() = Some(present_frame(
                    area,
                    &mut presenter.borrow_mut(),
                    frame,
                    *expected,
                ));
            }
            gtk::glib::Propagation::Stop
        });
        let mut pointers = std::collections::HashSet::new();
        let mut reused = false;
        // Keep GTK and the worker alive across uploads: a single-frame check
        // cannot exercise output reuse or the previous CUDA copy's source lease.
        for index in 0..12 {
            let color = index % PREVIEW_COLORS.len();
            let position = Time::from_seconds(color as i64);
            commands.send(VideoCommand::Render {
                position,
                accuracy: CompositeAccuracy::FULLY_ACCURATE,
            })?;
            let frame = loop {
                match events.recv_timeout(Duration::from_secs(180))? {
                    VideoEvent::Frame {
                        frame,
                        position: rendered,
                        ..
                    } if rendered == position => break frame,
                    VideoEvent::Error(error) => return Err(error.into()),
                    _ => {}
                }
            };
            if !gpu::intel_backend_selected() {
                reused |= !pointers.insert(frame.buffer.cuda()?.cu_deviceptr());
            }
            *current_frame.borrow_mut() = Some((frame, PREVIEW_COLORS[color]));
            *result.borrow_mut() = None;
            area.queue_render();
            while result.borrow().is_none() {
                context.iteration(true);
            }
            result.borrow_mut().take().expect("GTK render result")?;
        }
        if !gpu::intel_backend_selected() && !reused {
            return Err("CUDA preview never reused a completed output buffer".into());
        }
        commands.send(VideoCommand::Stop)?;
        loop {
            match events.recv_timeout(Duration::from_secs(180)) {
                Ok(VideoEvent::Error(error)) => return Err(error.into()),
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(error) => return Err(error.into()),
            }
        }
        area.make_current();
        renderer.borrow_mut().destroy();
        println!("GTK displayed all 12 alternating GPU frames through native texture sharing");
        window.close();
        println!("Restarted backend initialized: {selected:?}");
        return Ok(());
    }
    // Never overwrite an existing user's preferences during hardware validation.
    let directory = std::path::PathBuf::from(
        std::env::var_os("XDG_CONFIG_HOME")
            .ok_or("Set XDG_CONFIG_HOME to a new empty validation directory")?,
    );
    if !directory.is_dir() || directory.read_dir()?.next().is_some() {
        return Err(
            "GPU preferences validation requires an empty XDG_CONFIG_HOME directory".into(),
        );
    }
    gtk::init()?;
    adw::init()?;
    let store = preferences::open_with_defaults();
    let window = adw::ApplicationWindow::builder()
        .default_width(800)
        .default_height(600)
        .build();
    window.present();
    shrimply_preferences_gtk::page::show_preferences_dialog(&window, store.clone());
    let dialog = window
        .visible_dialog()
        .ok_or("Preferences dialog was not presented")?
        .downcast::<adw::PreferencesDialog>()
        .map_err(|_| "Wrong preferences dialog type")?;
    dialog.set_visible_page_name("performance");
    let row = acceleration_row(dialog.upcast_ref())
        .ok_or("Performance page omitted its acceleration ComboRow")?;
    let devices = gpu::acceleration_devices()?;
    let ids = std::iter::once(String::new())
        .chain(devices.iter().map(|device| device.id.clone()))
        .collect::<Vec<_>>();
    let model = row.model().ok_or("Acceleration ComboRow has no model")?;
    if devices.len() < 2 || model.n_items() as usize != ids.len() {
        return Err(format!(
            "Hybrid GPU validation needs both Intel and NVIDIA; found {} devices",
            devices.len()
        )
        .into());
    }
    for (index, id) in ids.iter().enumerate() {
        row.set_selected(index as u32);
        if preferences::snapshot(&store).acceleration_device != *id {
            return Err("Acceleration ComboRow did not save its selection".into());
        }
        let label = model
            .item(index as u32)
            .ok_or("Missing GPU label")?
            .downcast::<gtk::StringObject>()
            .map_err(|_| "GPU label is not a string")?;
        println!("ComboRow selection: {} = {id:?}", label.string());
        let result = std::process::Command::new(std::env::current_exe()?)
            .arg(id)
            .env_remove("SHRIMPLY_GPU")
            .status()?;
        if !result.success() {
            return Err(format!("Restart failed for acceleration device {id:?}").into());
        }
    }
    row.set_selected(0);
    dialog.close();
    window.close();
    println!(
        "Hybrid GTK acceleration ComboRow persistence and restarted GPU initialization passed"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
const PREVIEW_COLORS: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];

#[cfg(target_os = "linux")]
fn preview_worker() -> (
    shrimply_visual_cuda::compositor::VideoCommandSender,
    std::sync::mpsc::Receiver<shrimply_visual_cuda::compositor::VideoEvent>,
) {
    use shrimply_project_document::project::*;
    let size = CanvasSize {
        width: 64,
        height: 64,
    };
    let items = PREVIEW_COLORS
        .into_iter()
        .enumerate()
        .map(|(index, color)| {
            let mut item = VideoItem::background_item(
                size,
                Time::from_seconds(index as i64),
                Time::from_seconds(index as i64 + 1),
            );
            let VideoItemContent::Background(background) = &mut item.content else {
                unreachable!();
            };
            let BackgroundGenerator::SolidColor(solid) = &mut background.generator else {
                unreachable!();
            };
            solid.color = shrimply_preview_gtk::timeline_value::TimelineValue::new_const(
                Color::from_rgba(color[0], color[1], color[2], color[3]),
            );
            item
        })
        .collect();
    shrimply_visual_cuda::compositor::spawn_worker(Project {
        canvas_size: size,
        video_tracks: vec![VideoTrack {
            items,
            ..Default::default()
        }],
        ..Default::default()
    })
}

#[cfg(target_os = "linux")]
fn present_frame(
    area: &gtk::GLArea,
    renderer: &mut shrimply_preview_runtime_cuda::renderer::VideoRenderer,
    frame: &gpu::CompositedVideoFrame,
    expected: [u8; 4],
) -> Result<(), String> {
    use glow::HasContext;
    use shrimply_preview_runtime_cuda::{
        Color,
        renderer::Appearance,
        timeline::renderer::{Rect, vec2},
    };
    let width = area.width();
    let height = area.height();
    let rect = Rect::from_min_max(vec2(0.0, 0.0), vec2(width as f32, height as f32));
    let result = renderer.render(
        glam::IVec2::new(width, height),
        1.0,
        Some(frame),
        Appearance {
            content_rect: rect,
            clip_rect: rect,
            shadow_size_px: 0,
            background_color: Color::BLACK,
            upsample_method: preferences::PreviewUpsampleMethod::Nearest,
            downsample_method: preferences::PreviewDownsampleMethod::Nearest,
        },
        |_| {},
    );
    let gl = shrimply_components_gtk::gl_loader::context();
    let mut pixel = [0; 4];
    unsafe {
        gl.read_pixels(
            width / 2,
            height / 2,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
    }
    result?;
    if pixel != expected {
        return Err(format!(
            "GTK GPU preview pixel mismatch: {pixel:?}; expected {expected:?}"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn acceleration_row(widget: &gtk::Widget) -> Option<adw::ComboRow> {
    if let Some(row) = widget.downcast_ref::<adw::ComboRow>()
        && row.title() == "Acceleration device"
    {
        return Some(row.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(row) = acceleration_row(&widget) {
            return Some(row);
        }
        child = widget.next_sibling();
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("GTK hybrid GPU validation requires Linux");
}
