use crate::gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext, ModifierModule};

pub(crate) use shrimply_visual_core::transparent_fill::{
    render_position, snapped_transparent_fill_position, validate_cache,
};

impl GpuModifier for shrimply_visual_core::transparent_fill::ResolvedMask {
    fn name(&self) -> &'static str {
        "Transparent Fill"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let width = input.width();
        let height = input.height();
        let Some(mask) = &self.mask else {
            return Ok(input);
        };
        let mask = context.upload_transparent_fill_mask(mask)?;
        let count = width as usize * height as usize;
        let mut pass = input.into_pass(context)?;
        let module = context.modifier_module(ModifierModule::Matte)?;
        unsafe {
            shrimply_gpu_cuda::cuda_launch! {
                kernel: transparent_fill_apply_mask,
                stream: context.stream(), module: &module,
                config: shrimply_gpu_cuda::LaunchConfig::for_num_elems(u32::try_from(count).map_err(|_| "canvas is too large")?),
                args: [pass.input_ptr(), mask, slice_mut(pass.output_buffer()), shrimply_render_core::TransparentFillMaskParams {
                    width,
                    height,
                    stride: width.div_ceil(8),
                }]
            }
        }
        .map_err(|error| format!("launch transparent fill mask kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

#[cfg(test)]
mod tests {
    use shrimply_visual_core::transparent_fill::{
        CACHE_VERSION, MEMORY_FRAMES, TransparentFillMaskCache, cache_key, frame_count,
    };
    use shrimply_visual_modifiers::transparent_fill::TransparentFillModifier;
    use std::{
        env, fs,
        io::BufWriter,
        path::{Path, PathBuf},
        process::Command,
        thread,
        time::{Duration, Instant},
    };
    use uuid::Uuid;

    use glam::Vec2;
    use shrimply_asset::Asset;
    use shrimply_gpu_cuda::CudaContext;
    use shrimply_gpu_cuda_memory::AllocationClass;
    use shrimply_math_core::{Time, fraction_new};
    use shrimply_project_document::project::{
        CanvasSize, ItemAddress, Project, VideoItem, VideoItemContent, VideoTrack, VisualModifier,
        activate_project, create_project_file, prepare_project, shutdown_history,
    };
    use shrimply_property_model::timeline_value::TimelineValue;
    use shrimply_visual_modifiers::{
        ModifierEffect, RasterModifierEffect, color_correction::ColorCorrectionModifier,
    };

    use super::*;
    use crate::{
        compositor::{
            CompositeAccuracy, VideoCommand, VideoEvent, VideoExportRenderer, spawn_worker,
        },
        transparent_fill_analysis::{self, Status},
    };

    const WIDTH: u32 = 96;
    const HEIGHT: u32 = 64;
    const FIRST_FRAME: u64 = 5_895;
    const FRAME_COUNT: u64 = 15;
    const E2E_WIDTH: u32 = 64;
    const E2E_HEIGHT: u32 = 64;
    const E2E_FIRST_PROJECT_FRAME: u64 = 5_895;
    const E2E_PROJECT_FRAMES: u64 = 31;
    const E2E_SOURCE_FPS: i64 = 24;
    const E2E_PROJECT_FPS: i64 = 30;
    const E2E_SOURCE_OFFSET_NUMERATOR: i64 = 103_101_571;
    const E2E_SOURCE_OFFSET_DENOMINATOR: i64 = 50_000_000;
    const E2E_SQUARE_SIZE: u32 = 16;
    const E2E_SQUARE_STEP: u32 = 4;
    const E2E_SQUARE_RANGE: u32 = E2E_WIDTH - E2E_SQUARE_SIZE;

    #[test]
    fn partial_first_project_frame_uses_the_item_start_mask() {
        let mut project: Project =
            serde_json::from_str("{}").expect("create partial-frame project");
        project.fps = fraction_new(30, 1);
        let start = Time::from_fraction(1, 120);
        let end = Time::from_fraction(1, 20);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Raster(Box::new(
                RasterModifierEffect::TransparentFill(TransparentFillModifier {
                    points: vec![
                        shrimply_visual_modifiers::transparent_fill::TransparentFillPoint {
                            id: Uuid::new_v4(),
                            position: TimelineValue::new_const(Vec2::splat(0.5)),
                        },
                    ],
                    tolerance: TimelineValue::new_const(0.1),
                    maximum_gap: 0,
                    analysis_generation: 1,
                }),
            ))));

        assert_eq!(frame_count(&project, &item), Some(2));
        assert_eq!(
            render_position(&project, &item, Time::from_fraction(1, 60)),
            start
        );
    }

    #[test]
    fn generates_transparent_fill_end_to_end_fixture() {
        let fixture = transparent_fill_fixture();
        assert!(fixture.video.exists());
        assert!(fixture.project_path.exists());
    }

    #[test]
    #[ignore = "requires enough free GPU memory for Transparent Fill analysis and export"]
    fn transparent_fill_analyzes_and_renders_a_real_project_end_to_end() {
        let fixture = transparent_fill_fixture();
        let original = env::current_dir().expect("read end-to-end test working directory");
        env::set_current_dir(&fixture.directory)
            .expect("isolate end-to-end Transparent Fill cache");
        ffmpeg_next::init().expect("initialize FFmpeg");

        let prepared = prepare_project(&fixture.project_path).expect("load generated project");
        let project = activate_project(prepared);
        transparent_fill_analysis::analyze(project.clone(), &fixture.address, fixture.modifier_id)
            .expect("start real Transparent Fill analysis");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match transparent_fill_analysis::status(&project, &fixture.address, fixture.modifier_id)
            {
                Status::Complete => break,
                Status::Failed(error) => panic!("Transparent Fill analysis failed: {error}"),
                Status::Cancelled => panic!("Transparent Fill analysis was cancelled"),
                Status::Missing | Status::Running { .. } if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                status => panic!("Transparent Fill analysis timed out with status {status:?}"),
            }
        }
        validate_cache(&project).expect("validate analyzed Transparent Fill cache");

        fs::create_dir_all(&fixture.rendered).expect("create rendered fixture directory");
        let mut renderer = VideoExportRenderer::new(48_000).expect("create export renderer");
        for frame_index in E2E_FIRST_PROJECT_FRAME..E2E_FIRST_PROJECT_FRAME + E2E_PROJECT_FRAMES {
            let position =
                shrimply_math_core::time_from_frame(frame_index, fraction_new(E2E_PROJECT_FPS, 1))
                    .expect("end-to-end project frame position");
            let composited = renderer
                .render(&project, position, 0)
                .unwrap_or_else(|error| panic!("render project frame {frame_index}: {error}"));
            let mut rgba = ffmpeg_next::frame::Video::new(
                ffmpeg_next::format::Pixel::RGBA,
                E2E_WIDTH,
                E2E_HEIGHT,
            );
            renderer
                .copy_to_rgba_frame(composited, &mut rgba)
                .unwrap_or_else(|error| panic!("copy project frame {frame_index}: {error}"));
            let pixels = compact_rgba(&rgba);
            assert_transparent_fill_frame(&pixels, frame_index);
            write_rgba_png(
                &fixture.rendered.join(format!("frame-{frame_index:02}.png")),
                &pixels,
            );
        }
        renderer.shutdown();
        shutdown_history().expect("stop generated project history");
        env::set_current_dir(original).expect("restore end-to-end test working directory");
    }

    #[test]
    fn cache_round_trips_evicted_project_frame_masks() {
        let directory = tempfile::tempdir().expect("create cache round-trip directory");
        const FRAMES: u64 = MEMORY_FRAMES as u64 + 16;
        let modifier_id = Uuid::new_v4();
        let key = format!("{CACHE_VERSION}:{modifier_id}:test");
        let cache = TransparentFillMaskCache::open(&directory.path().join("masks.sqlite"))
            .expect("open cache round-trip database");
        cache.begin_analysis(&key).expect("begin cache round-trip");
        for frame in 0..FRAMES {
            let mut mask = vec![0_u8; WIDTH.div_ceil(8) as usize * HEIGHT as usize];
            let marker = (frame % u64::from(WIDTH)) as u32;
            for row in mask.chunks_exact_mut(WIDTH.div_ceil(8) as usize) {
                row[marker as usize / 8] |= 0x80 >> (marker % 8);
            }
            cache
                .insert_staged(&key, frame as i64, &mask, WIDTH, HEIGHT)
                .expect("insert cache round-trip frame");
        }
        cache
            .complete_analysis(&key, WIDTH, HEIGHT, FRAMES)
            .expect("complete cache round-trip");

        for frame in 0..FRAMES {
            let mask = cache
                .get(&key, frame as i64, WIDTH, HEIGHT)
                .expect("read cache round-trip frame")
                .expect("cache round-trip frame exists");
            let marker = (frame % u64::from(WIDTH)) as usize;
            assert_ne!(mask[marker / 8] & (0x80 >> (marker % 8)), 0);
        }
        cache.abort_analysis(&key);
    }

    #[test]
    fn cached_mask_applies_with_the_cuda_kernel() {
        const FRAME: i64 = 5_897;
        const MARKER: u32 = 17;
        let directory = tempfile::tempdir().expect("create CUDA mask test directory");
        let modifier_id = Uuid::new_v4();
        let key = format!("{CACHE_VERSION}:{modifier_id}:cuda-test");
        let cache = TransparentFillMaskCache::open(&directory.path().join("masks.sqlite"))
            .expect("open CUDA mask test cache");
        cache
            .begin_analysis(&key)
            .expect("begin CUDA mask test analysis");
        let mut packed = vec![0_u8; WIDTH.div_ceil(8) as usize * HEIGHT as usize];
        for row in packed.chunks_exact_mut(WIDTH.div_ceil(8) as usize) {
            row[MARKER as usize / 8] |= 0x80 >> (MARKER % 8);
        }
        cache
            .insert_staged(&key, FRAME, &packed, WIDTH, HEIGHT)
            .expect("insert CUDA mask test frame");
        cache
            .complete_analysis(&key, WIDTH, HEIGHT, 1)
            .expect("complete CUDA mask test analysis");
        let mask = cache
            .get(&key, FRAME, WIDTH, HEIGHT)
            .expect("read CUDA mask test frame")
            .expect("CUDA mask test frame exists");

        let context = CudaContext::new(0).expect("create CUDA mask test context");
        let stream = context.new_stream().expect("create CUDA mask test stream");
        let module = context
            .load_module_from_image(ModifierModule::Matte.image())
            .expect("load CUDA matte module");
        let pixels = WIDTH as usize * HEIGHT as usize;
        let input_pixels = vec![u32::MAX; pixels];
        let mut input = shrimply_gpu_cuda_memory::global()
            .allocate_buffer(
                &stream,
                pixels,
                AllocationClass::Transient,
                "CUDA mask input",
            )
            .expect("allocate CUDA mask input");
        input
            .copy_from_host(&stream, &input_pixels)
            .expect("upload CUDA mask input");
        let mut device_mask = shrimply_gpu_cuda_memory::global()
            .allocate_buffer(
                &stream,
                mask.len(),
                AllocationClass::Transient,
                "CUDA mask upload",
            )
            .expect("allocate CUDA mask");
        device_mask
            .copy_from_host(&stream, &mask)
            .expect("upload CUDA mask");
        let mut output = shrimply_gpu_cuda_memory::global()
            .allocate_buffer::<u32>(
                &stream,
                pixels,
                AllocationClass::Transient,
                "CUDA mask output",
            )
            .expect("allocate CUDA mask output");
        unsafe {
            shrimply_gpu_cuda::cuda_launch! {
                kernel: transparent_fill_apply_mask,
                stream: &stream,
                module: &module,
                config: shrimply_gpu_cuda::LaunchConfig::for_num_elems(pixels as u32),
                args: [
                    input.cu_deviceptr() as usize as *const u32,
                    device_mask.cu_deviceptr() as usize as *const u8,
                    slice_mut(&mut output),
                    shrimply_render_core::TransparentFillMaskParams {
                        width: WIDTH,
                        height: HEIGHT,
                        stride: WIDTH.div_ceil(8),
                    }
                ]
            }
        }
        .expect("launch Transparent Fill CUDA kernel");
        let output = output
            .to_host_vec(&stream)
            .expect("download CUDA mask output");
        let row = HEIGHT as usize / 2;
        assert_eq!(output[row * WIDTH as usize + MARKER as usize], 0);
        assert_eq!(output[row * WIDTH as usize + MARKER as usize + 1], u32::MAX);
        cache.abort_analysis(&key);
    }

    #[test]
    #[ignore = "requires enough free GPU memory for an independent preview compositor"]
    fn preview_compositor_applies_each_out_of_order_project_frame_mask() {
        let directory = tempfile::tempdir().expect("create preview mask test directory");
        let original = env::current_dir().expect("read preview mask test working directory");
        env::set_current_dir(directory.path()).expect("isolate preview mask test cache database");
        let mut project: Project = serde_json::from_str("{}").expect("create default project");
        project.fps = fraction_new(30, 1);
        project.canvas_size = CanvasSize {
            width: WIDTH,
            height: HEIGHT,
        };
        let start = Time::from_fraction(393, 2);
        let end = Time::from_seconds(197);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.source_duration = end.saturating_sub(start);
        let fill = TransparentFillModifier {
            points: vec![
                shrimply_visual_modifiers::transparent_fill::TransparentFillPoint {
                    id: Uuid::new_v4(),
                    position: TimelineValue::new_const(Vec2::splat(0.5)),
                },
            ],
            tolerance: TimelineValue::new_const(0.1),
            maximum_gap: 0,
            analysis_generation: 1,
        };
        let modifier = VisualModifier::new(ModifierEffect::Raster(Box::new(
            RasterModifierEffect::TransparentFill(fill.clone()),
        )));
        let modifier_id = modifier.id;
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Rasterize(
                Default::default(),
            )));
        item.modifiers.push(modifier);
        project.video_tracks.push(VideoTrack {
            items: vec![item],
            ..Default::default()
        });
        let item = &project.video_tracks[0].items[0];
        let key = cache_key(&project, item, modifier_id, 1, &fill);
        let cache = TransparentFillMaskCache::shared();
        cache
            .begin_analysis(&key)
            .expect("begin preview mask test analysis");
        insert_marker_masks(&cache, &key);

        let (commands, events) = spawn_worker(project);
        for frame in [5_897, 5_895, 5_899, 5_896, 5_902, 5_898, 5_909, 5_900] {
            let position = shrimply_math_core::time_from_frame(frame, fraction_new(30, 1))
                .expect("project frame position")
                .saturating_add(Time::from_fraction(1, 180));
            commands
                .send(VideoCommand::Render {
                    position,
                    accuracy: CompositeAccuracy::FULLY_ACCURATE,
                })
                .expect("request preview mask frame");
            loop {
                match events
                    .recv_timeout(Duration::from_secs(10))
                    .expect("receive preview mask frame")
                {
                    VideoEvent::Frame {
                        frame: output,
                        position: rendered_position,
                        settled,
                        ..
                    } if rendered_position == position && settled => {
                        assert_marker(output, frame);
                        break;
                    }
                    VideoEvent::Loading {
                        position: loading, ..
                    } if loading == position => commands
                        .send(VideoCommand::Render {
                            position,
                            accuracy: CompositeAccuracy::FULLY_ACCURATE,
                        })
                        .expect("retry loading preview mask frame"),
                    VideoEvent::Error(error) => panic!("preview mask test failed: {error}"),
                    _ => {}
                }
            }
        }
        commands
            .send(VideoCommand::Stop)
            .expect("stop preview mask worker");
        drop(commands);
        while events.recv().is_ok() {}
        cache.abort_analysis(&key);
        env::set_current_dir(original).expect("restore preview mask test working directory");
    }

    #[test]
    #[ignore = "requires enough free GPU memory for an independent preview compositor"]
    fn preview_uses_the_mask_for_each_project_frame() {
        let directory = tempfile::tempdir().expect("create transparent fill test directory");
        let original = env::current_dir().expect("read playback test working directory");
        env::set_current_dir(directory.path()).expect("isolate playback test cache database");
        let video = directory.path().join("24fps.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=96x64:r=24:d=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&video)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg failed to generate the test video");

        let mut project: Project = serde_json::from_str("{}").expect("create default project");
        project.fps = fraction_new(30, 1);
        project.canvas_size = CanvasSize {
            width: WIDTH,
            height: HEIGHT,
        };
        let start = Time::from_fraction(393, 2);
        let end = Time::from_seconds(197);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.content = VideoItemContent::Media;
        item.file = Asset::new(video);
        item.source_width = WIDTH;
        item.source_height = HEIGHT;
        item.source_duration = Time::from_seconds(1);
        item.playback_fps = fraction_new(24, 1);

        let fill = TransparentFillModifier {
            points: vec![
                shrimply_visual_modifiers::transparent_fill::TransparentFillPoint {
                    id: Uuid::new_v4(),
                    position: TimelineValue::new_const(Vec2::splat(0.5)),
                },
            ],
            tolerance: TimelineValue::new_const(0.1),
            maximum_gap: 0,
            analysis_generation: 1,
        };
        let modifier = VisualModifier::new(ModifierEffect::Raster(Box::new(
            RasterModifierEffect::TransparentFill(fill.clone()),
        )));
        let modifier_id = modifier.id;
        item.modifiers.push(modifier);
        let track = VideoTrack {
            items: vec![item],
            ..Default::default()
        };
        project.video_tracks.push(track);

        let item = &project.video_tracks[0].items[0];
        let key = cache_key(&project, item, modifier_id, 0, &fill);
        let cache = TransparentFillMaskCache::shared();
        cache
            .begin_analysis(&key)
            .expect("begin test mask analysis");
        insert_marker_masks(&cache, &key);

        let (commands, events) = spawn_worker(project);
        let order = [
            5_897, 5_895, 5_899, 5_896, 5_902, 5_898, 5_909, 5_900, 5_908, 5_901, 5_907, 5_903,
            5_906, 5_904, 5_905,
        ];
        for accuracy in [
            CompositeAccuracy::TIME_ACCURATE,
            CompositeAccuracy::FULLY_ACCURATE,
            CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
        ] {
            for frame in order {
                let position = shrimply_math_core::time_from_frame(frame, fraction_new(30, 1))
                    .expect("project frame position")
                    .saturating_add(Time::from_fraction(1, 180));
                commands
                    .send(VideoCommand::Render { position, accuracy })
                    .expect("request preview frame");
                loop {
                    match events
                        .recv_timeout(Duration::from_secs(10))
                        .expect("receive preview frame")
                    {
                        VideoEvent::Frame {
                            frame: output,
                            position: rendered_position,
                            settled,
                            ..
                        } if rendered_position == position && settled => {
                            assert_marker(output, frame);
                            break;
                        }
                        VideoEvent::Loading {
                            position: loading, ..
                        } if loading == position => {
                            commands
                                .send(VideoCommand::Render { position, accuracy })
                                .expect("retry loading preview frame");
                        }
                        VideoEvent::Error(error) => panic!("preview failed: {error}"),
                        _ => {}
                    }
                }
            }
        }
        commands
            .send(VideoCommand::Stop)
            .expect("stop preview worker");
        drop(commands);
        while events.recv().is_ok() {}
        cache.abort_analysis(&key);
        env::set_current_dir(original).expect("restore playback test working directory");
    }

    struct EndToEndFixture {
        directory: PathBuf,
        video: PathBuf,
        project_path: PathBuf,
        rendered: PathBuf,
        address: ItemAddress,
        modifier_id: Uuid,
    }

    fn transparent_fill_fixture() -> EndToEndFixture {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../video")
            .join("target/transparent-fill-e2e");
        fs::create_dir_all(&directory).expect("create Transparent Fill fixture directory");
        let video = directory.join("moving-square-24fps.mp4");
        let filter = format!(
            "[0:v][1:v]overlay=x='mod({E2E_SQUARE_STEP}*n,{E2E_SQUARE_RANGE})':y=24:eval=frame:shortest=1"
        );
        let status = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=64x64:r=24:d=4",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=16x16:r=24:d=4",
                "-filter_complex",
                &filter,
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&video)
            .status()
            .expect("run FFmpeg for Transparent Fill fixture");
        assert!(
            status.success(),
            "FFmpeg failed to generate moving square video"
        );

        let mut project: Project = serde_json::from_str("{}").expect("create fixture project");
        project.name = "Transparent Fill end-to-end".to_string();
        project.fps = fraction_new(E2E_PROJECT_FPS, 1);
        project.canvas_size = CanvasSize {
            width: E2E_WIDTH,
            height: E2E_HEIGHT,
        };
        let start = Time::from_fraction(393, 2);
        let end = Time::from_fraction(4_938, 25);
        project.cursor_position = Some(start);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.content = VideoItemContent::Media;
        item.file = Asset::new(video.clone());
        item.source_width = E2E_WIDTH;
        item.source_height = E2E_HEIGHT;
        item.source_duration = Time::from_seconds(4);
        item.time_offset =
            Time::from_fraction(E2E_SOURCE_OFFSET_NUMERATOR, E2E_SOURCE_OFFSET_DENOMINATOR);
        item.playback_fps = fraction_new(E2E_SOURCE_FPS, 1);

        let color_correction = ColorCorrectionModifier {
            brightness: TimelineValue::new_const(-0.05),
            ..Default::default()
        };
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Raster(Box::new(
                RasterModifierEffect::ColorCorrection(Box::new(color_correction)),
            ))));
        let fill = TransparentFillModifier {
            points: vec![
                shrimply_visual_modifiers::transparent_fill::TransparentFillPoint {
                    id: Uuid::new_v4(),
                    position: TimelineValue::new_const(Vec2::splat(0.05)),
                },
            ],
            tolerance: TimelineValue::new_const(0.12),
            maximum_gap: 2,
            analysis_generation: 1,
        };
        let fill_modifier = VisualModifier::new(ModifierEffect::Raster(Box::new(
            RasterModifierEffect::TransparentFill(fill),
        )));
        let modifier_id = fill_modifier.id;
        item.modifiers.push(fill_modifier);
        let item_id = item.id;
        let track = VideoTrack {
            items: vec![item],
            ..Default::default()
        };
        let track_id = track.id;
        project.video_tracks.push(track);

        let project_path = directory.join("transparent-fill-e2e.shrimp");
        create_project_file(&project_path, &project)
            .expect("write Transparent Fill fixture project");
        EndToEndFixture {
            rendered: directory.join("rendered-alpha"),
            directory,
            video,
            project_path,
            address: ItemAddress::Video {
                sequence_path: Vec::new(),
                track_id,
                item_id,
            },
            modifier_id,
        }
    }

    fn compact_rgba(frame: &ffmpeg_next::frame::Video) -> Vec<u8> {
        let row_bytes = E2E_WIDTH as usize * 4;
        frame
            .data(0)
            .chunks_exact(frame.stride(0))
            .take(E2E_HEIGHT as usize)
            .flat_map(|row| row[..row_bytes].iter().copied())
            .collect()
    }

    fn assert_transparent_fill_frame(pixels: &[u8], project_frame: u64) {
        let project_position =
            shrimply_math_core::time_from_frame(project_frame, fraction_new(E2E_PROJECT_FPS, 1))
                .expect("end-to-end assertion project position");
        let source_position =
            Time::from_fraction(E2E_SOURCE_OFFSET_NUMERATOR, E2E_SOURCE_OFFSET_DENOMINATOR)
                .saturating_add(project_position.saturating_sub(Time::from_fraction(393, 2)));
        let source_frame =
            shrimply_math_core::frame_index(source_position, fraction_new(E2E_SOURCE_FPS, 1))
                .and_then(|frame| u64::try_from(frame).ok())
                .expect("end-to-end assertion source frame");
        let square_x = ((source_frame as u32 + 1) * E2E_SQUARE_STEP) % E2E_SQUARE_RANGE;
        let alpha = |x: u32, y: u32| pixels[((y * E2E_WIDTH + x) * 4 + 3) as usize];
        assert_eq!(
            alpha(2, 2),
            0,
            "project frame {project_frame} kept the white background opaque"
        );
        for x in [square_x + 3, square_x + E2E_SQUARE_SIZE - 4] {
            assert_eq!(
                alpha(x, 24 + E2E_SQUARE_SIZE / 2),
                u8::MAX,
                "project frame {project_frame} used a mask from a different source frame"
            );
        }
    }

    fn write_rgba_png(path: &Path, pixels: &[u8]) {
        let output = fs::File::create(path).expect("create rendered Transparent Fill PNG");
        let mut encoder = png::Encoder::new(BufWriter::new(output), E2E_WIDTH, E2E_HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("write rendered Transparent Fill PNG header")
            .write_image_data(pixels)
            .expect("write rendered Transparent Fill PNG pixels");
    }

    fn marker(frame: u64) -> u32 {
        ((frame - FIRST_FRAME) * 5 % u64::from(WIDTH - 1)) as u32
    }

    fn insert_marker_masks(cache: &TransparentFillMaskCache, key: &str) {
        for frame in FIRST_FRAME..FIRST_FRAME + FRAME_COUNT {
            let marker = marker(frame);
            let mut mask = vec![0_u8; WIDTH.div_ceil(8) as usize * HEIGHT as usize];
            for row in mask.chunks_exact_mut(WIDTH.div_ceil(8) as usize) {
                row[marker as usize / 8] |= 0x80 >> (marker % 8);
            }
            cache
                .insert_staged(key, frame as i64, &mask, WIDTH, HEIGHT)
                .expect("insert test frame mask");
        }
        cache
            .complete_analysis(key, WIDTH, HEIGHT, FRAME_COUNT)
            .expect("complete test mask analysis");
    }

    fn assert_marker(output: crate::gpu::CompositedVideoFrame, frame: u64) {
        let buffer = output.buffer.cuda().expect("CUDA preview frame");
        buffer
            .context()
            .synchronize()
            .expect("synchronize preview frame");
        let stream = buffer.context().default_stream();
        let pixels = buffer.to_host_vec(&stream).expect("download preview frame");
        let expected = marker(frame) as usize;
        let row = HEIGHT as usize / 2;
        let transparent: Vec<_> = pixels[row * WIDTH as usize..(row + 1) * WIDTH as usize]
            .iter()
            .enumerate()
            .filter_map(|(x, pixel)| (*pixel == 0).then_some(x))
            .collect();
        assert_eq!(
            transparent,
            vec![expected],
            "project frame {frame} did not use only its cached mask"
        );
    }
}
