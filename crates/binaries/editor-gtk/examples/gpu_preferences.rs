#![cfg(target_os = "linux")]

use adw::prelude::*;
use shrimply_preferences_core as preferences;
use shrimply_visual_cuda::gpu;
use std::{cell::RefCell, rc::Rc, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
        let frame = preview_frame()?;
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
        let result = Rc::new(RefCell::new(None));
        let rendered = result.clone();
        area.connect_render(move |area, _| {
            *rendered.borrow_mut() = Some(present_frame(area, &frame));
            gtk::glib::Propagation::Stop
        });
        area.queue_render();
        while result.borrow().is_none() {
            context.iteration(true);
        }
        result.borrow_mut().take().expect("GTK render result")?;
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

fn preview_frame() -> Result<gpu::CompositedVideoFrame, String> {
    use shrimply_project_document::project::*;
    use shrimply_visual_cuda::compositor::{
        CompositeAccuracy, VideoCommand, VideoEvent, spawn_worker,
    };
    let size = CanvasSize {
        width: 64,
        height: 64,
    };
    let mut item = VideoItem::background_item(size, Time::ZERO, Time::from_seconds(1));
    let VideoItemContent::Background(background) = &mut item.content else {
        unreachable!();
    };
    let BackgroundGenerator::SolidColor(solid) = &mut background.generator else {
        unreachable!();
    };
    solid.color = shrimply_preview_gtk::timeline_value::TimelineValue::new_const(Color::from_rgba(
        255, 0, 0, 255,
    ));
    let (commands, events) = spawn_worker(Project {
        canvas_size: size,
        video_tracks: vec![VideoTrack {
            items: vec![item],
            ..Default::default()
        }],
        ..Default::default()
    });
    commands
        .send(VideoCommand::Render {
            position: Time::ZERO,
            accuracy: CompositeAccuracy::FULLY_ACCURATE,
        })
        .map_err(|error| error.to_string())?;
    let frame = loop {
        match events
            .recv_timeout(Duration::from_secs(180))
            .map_err(|error| error.to_string())?
        {
            VideoEvent::Frame { frame, .. } => break frame,
            VideoEvent::Error(error) => return Err(error),
            _ => {}
        }
    };
    commands
        .send(VideoCommand::Stop)
        .map_err(|error| error.to_string())?;
    loop {
        match events.recv_timeout(Duration::from_secs(180)) {
            Ok(VideoEvent::Error(error)) => return Err(error),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(frame)
}

fn present_frame(area: &gtk::GLArea, frame: &gpu::CompositedVideoFrame) -> Result<(), String> {
    use glow::HasContext;
    use shrimply_preview_runtime_cuda::{
        Color,
        renderer::{Appearance, VideoRenderer},
        timeline::renderer::{Rect, vec2},
    };
    let width = area.width();
    let height = area.height();
    let rect = Rect::from_min_max(vec2(0.0, 0.0), vec2(width as f32, height as f32));
    let mut renderer = VideoRenderer::new()?;
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
    renderer.destroy();
    result?;
    if pixel != [255, 0, 0, 255] {
        return Err(format!("GTK GPU preview pixel mismatch: {pixel:?}"));
    }
    println!("GTK displayed the GPU-rendered red frame through native texture sharing");
    Ok(())
}

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
