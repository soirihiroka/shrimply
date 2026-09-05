use std::rc::Rc;

use shrimply_math_geometry::ComposedTransform2D;

use crate::gpu::modifiers::GpuModifier;
use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::visual_bounds::ResolvedVisualBounds;
use crate::visual_source::VisualSourceCache;
use shrimply_project::project::{
    CanvasSize, LayerBlendMode, SkiaDrawingStrategy, TextureAddressMode, VideoSampleMethod,
};
use uuid::Uuid;

#[derive(Clone)]
pub enum VideoLayer {
    Nv12 {
        frame: VisualFrame,
        transform: ComposedTransform2D,
        motion_blur: Option<Rc<[ComposedTransform2D]>>,
        sample_method: VideoSampleMethod,
        compositing: ResolvedCompositing,
        crop: [f32; 4],
        padding: [f32; 4],
        address_mode: TextureAddressMode,
    },
    Rgba {
        layer: Rc<VisualFrame>,
        transform: ComposedTransform2D,
        motion_blur: Option<Rc<[ComposedTransform2D]>>,
        sample_method: VideoSampleMethod,
        compositing: ResolvedCompositing,
        crop: [f32; 4],
        padding: [f32; 4],
        address_mode: TextureAddressMode,
    },
}

#[derive(Clone, Copy)]
pub struct ResolvedCompositing {
    pub opacity: f32,
    pub blend_mode: LayerBlendMode,
}

#[derive(Clone, Copy)]
pub struct VisualState {
    pub transform: ComposedTransform2D,
    pub bounds: ResolvedVisualBounds,
    pub sampling: VideoSampleMethod,
    pub skia_drawing_strategy: SkiaDrawingStrategy,
    pub compositing: ResolvedCompositing,
}

impl VisualState {
    pub fn baked(self) -> Self {
        Self {
            transform: ComposedTransform2D::IDENTITY,
            bounds: ResolvedVisualBounds::default(),
            ..self
        }
    }
}

pub enum GpuFrame {
    Nv12(VisualFrame),
    Rgba(Rc<VisualFrame>),
}

pub(crate) trait VisualData {
    fn morph_scene(&self) -> Option<crate::vector_morph::MorphScene> {
        None
    }

    /// Draws this source and every queued vector operation into a project-sized Skia surface.
    ///
    /// `operations` are part of vector rendering, not post-raster layer state. In particular,
    /// transforms (including shear), opacity, repeats, and vector transitions must reach Skia so
    /// paths are transformed before rasterization. Do not rasterize the source alone and replay
    /// these operations on the returned texture; that silently reduces vector quality.
    fn rasterize(
        &self,
        compositor: &mut CudaVideoCompositor,
        drawing_strategy: SkiaDrawingStrategy,
        operations: &[VectorOperation],
    ) -> Result<Rc<VisualFrame>, String>;
}

pub use shrimply_video_core::generated::{TextMaskOperation, VectorOperation};

enum LazySource {
    Data(Box<dyn VisualData>),
    Frame(GpuFrame),
}

enum PlannedOperation {
    Spatial(Box<dyn FnOnce(&mut VisualState)>),
    Pixel(Box<dyn GpuModifier>),
    PreservingPixel(Box<dyn PreservingRasterModifier>),
    Rasterize(SkiaDrawingStrategy, VideoSampleMethod),
    MeshFlow {
        grid_width: u32,
        grid_height: u32,
        source_offsets: Vec<glam::Vec2>,
    },
    MotionBlur(Rc<[ComposedTransform2D]>),
    BeginAlphaMask(crate::alpha_mask::ResolvedShapeAlphaMask),
    EndAlphaMask,
}

pub(crate) trait PreservingRasterModifier {
    fn resolve(&self, state: VisualState) -> Box<dyn GpuModifier>;
}

/// Source data plus an ordered modifier plan. Folding this plan never allocates GPU pixels.
struct LazyVisual {
    source: LazySource,
    initial: VisualState,
    vector_operations: Vec<VectorOperation>,
    operations: Vec<PlannedOperation>,
}

pub struct VectorVisual(LazyVisual);

pub struct RasterVisual(LazyVisual);

pub enum Visual {
    Vector(VectorVisual),
    Raster(RasterVisual),
}

pub(crate) struct VectorMorphInput {
    pub scene: crate::vector_morph::MorphScene,
    pub state: VisualState,
}

impl VectorVisual {
    /// Starts a vector plan whose transform remains a Skia operation until rasterization.
    ///
    /// The source implementations render at project canvas size. Keeping the item transform in
    /// `vector_operations` is intentional: moving it back into `VisualState` would make affine
    /// transforms sample an already-rasterized texture.
    pub(crate) fn prepared(data: Box<dyn VisualData>, mut state: VisualState) -> Self {
        let transform = state.transform;
        state.transform = ComposedTransform2D::IDENTITY;
        state.bounds = ResolvedVisualBounds::default();
        Self(LazyVisual {
            source: LazySource::Data(data),
            initial: state,
            vector_operations: vec![VectorOperation::Transform(transform)],
            operations: Vec::new(),
        })
    }

    pub fn push(&mut self, operation: VectorOperation) {
        self.0.vector_operations.push(operation);
    }

    pub fn rasterize(
        mut self,
        drawing_strategy: SkiaDrawingStrategy,
        sample_method: VideoSampleMethod,
    ) -> RasterVisual {
        self.0
            .operations
            .push(PlannedOperation::Rasterize(drawing_strategy, sample_method));
        RasterVisual(self.0)
    }

    fn morph_input(&self) -> Option<VectorMorphInput> {
        if !self.0.operations.is_empty() {
            return None;
        }
        let LazySource::Data(data) = &self.0.source else {
            return None;
        };
        let scene = data.morph_scene().and_then(|scene| {
            crate::vector_morph::apply_vector_operations(scene, &self.0.vector_operations)
        })?;
        Some(VectorMorphInput {
            scene,
            state: self.0.initial,
        })
    }
}

impl RasterVisual {
    pub fn materialized(frame: GpuFrame, state: VisualState) -> Self {
        Self(LazyVisual {
            source: LazySource::Frame(frame),
            initial: state,
            vector_operations: Vec::new(),
            operations: Vec::new(),
        })
    }

    pub fn push_spatial(&mut self, operation: impl FnOnce(&mut VisualState) + 'static) {
        self.0
            .operations
            .push(PlannedOperation::Spatial(Box::new(operation)));
    }

    pub(crate) fn push_pixel(&mut self, modifier: Box<dyn GpuModifier>) {
        self.0.operations.push(PlannedOperation::Pixel(modifier));
    }

    pub(crate) fn push_preserving_pixel(&mut self, modifier: Box<dyn PreservingRasterModifier>) {
        self.0
            .operations
            .push(PlannedOperation::PreservingPixel(modifier));
    }

    pub(crate) fn push_mesh_flow(
        &mut self,
        grid_width: u32,
        grid_height: u32,
        source_offsets: Vec<glam::Vec2>,
    ) {
        self.0.operations.push(PlannedOperation::MeshFlow {
            grid_width,
            grid_height,
            source_offsets,
        });
    }

    fn push_motion_blur(&mut self, transforms: Rc<[ComposedTransform2D]>) {
        self.0
            .operations
            .push(PlannedOperation::MotionBlur(transforms));
    }

    fn begin_alpha_mask(&mut self, mask: crate::alpha_mask::ResolvedShapeAlphaMask) {
        self.0
            .operations
            .push(PlannedOperation::BeginAlphaMask(mask));
    }

    fn end_alpha_mask(&mut self) {
        self.0.operations.push(PlannedOperation::EndAlphaMask);
    }
}

impl Visual {
    pub(crate) fn morph_input(&self) -> Option<VectorMorphInput> {
        match self {
            Self::Vector(value) => value.morph_input(),
            Self::Raster(_) => None,
        }
    }

    pub fn rasterize(
        self,
        drawing_strategy: SkiaDrawingStrategy,
        sample_method: VideoSampleMethod,
    ) -> Self {
        match self {
            Self::Vector(value) => Self::Raster(value.rasterize(drawing_strategy, sample_method)),
            raster @ Self::Raster(_) => raster,
        }
    }

    pub fn push_transform(&mut self, transform: ComposedTransform2D) {
        match self {
            Self::Vector(value) => value.push(VectorOperation::Transform(transform)),
            Self::Raster(value) => value.push_spatial(move |state| {
                state.transform = transform.compose(state.transform);
            }),
        }
    }

    pub fn multiply_opacity(&mut self, opacity: f32) {
        match self {
            Self::Vector(value) => value.0.initial.compositing.opacity *= opacity,
            Self::Raster(value) => value.push_spatial(move |state| {
                state.compositing.opacity *= opacity;
            }),
        }
    }

    pub fn push_motion_blur(
        &mut self,
        current: ComposedTransform2D,
        samples: Vec<ComposedTransform2D>,
    ) {
        let Some(transforms) = shrimply_math_geometry::relative_motion_transforms(current, samples)
        else {
            return;
        };
        let transforms: Rc<[ComposedTransform2D]> = transforms.into();
        match self {
            Self::Vector(value) => value.push(VectorOperation::MotionBlur(transforms)),
            Self::Raster(value) => value.push_motion_blur(transforms),
        }
    }

    pub(crate) fn push_pixel(&mut self, modifier: Box<dyn GpuModifier>) {
        match self {
            Self::Vector(value) => value.0.operations.push(PlannedOperation::Pixel(modifier)),
            Self::Raster(value) => value.push_pixel(modifier),
        }
    }

    pub(crate) fn begin_alpha_mask(&mut self, mask: crate::alpha_mask::ResolvedShapeAlphaMask) {
        let Self::Raster(value) = self else {
            unreachable!("validated alpha mask received vector input")
        };
        value.begin_alpha_mask(mask);
    }

    pub(crate) fn end_alpha_mask(&mut self) {
        let Self::Raster(value) = self else {
            unreachable!("validated alpha mask received vector input")
        };
        value.end_alpha_mask();
    }

    pub(crate) fn push_alpha_mask(&mut self, mask: crate::alpha_mask::ResolvedShapeAlphaMask) {
        let Self::Raster(value) = self else {
            unreachable!("validated compositing alpha mask received vector input")
        };
        value.push_preserving_pixel(crate::alpha_mask::pending_shape(mask));
    }

    pub fn into_layer(
        self,
        compositor: &mut CudaVideoCompositor,
        canvas: CanvasSize,
        cache_scope: (&[Uuid], Uuid, Uuid),
        cache: &mut VisualSourceCache,
    ) -> Result<VideoLayer, String> {
        match self {
            Self::Vector(value) => value.0.into_layer(compositor, canvas, cache_scope, cache),
            Self::Raster(value) => value.0.into_layer(compositor, canvas, cache_scope, cache),
        }
    }
}

impl LazyVisual {
    fn into_layer(
        self,
        compositor: &mut CudaVideoCompositor,
        canvas: CanvasSize,
        cache_scope: (&[Uuid], Uuid, Uuid),
        cache: &mut VisualSourceCache,
    ) -> Result<VideoLayer, String> {
        let mut execution = ExecutionState::Prepared {
            source: self.source,
            state: self.initial,
            vector_operations: self.vector_operations,
        };
        let mut alpha_mask_branch = None;

        for operation in self.operations {
            match operation {
                PlannedOperation::Spatial(operation) => operation(execution.state_mut()),
                PlannedOperation::Pixel(modifier) => {
                    let (frame, state) = execution
                        .materialize(compositor, canvas, cache_scope, cache)?
                        .into_materialized();
                    let (frame, state) = apply_pixel(compositor, canvas, frame, state, &*modifier)?;
                    execution = ExecutionState::Materialized { frame, state };
                }
                PlannedOperation::PreservingPixel(modifier) => {
                    let (frame, state) = execution
                        .materialize(compositor, canvas, cache_scope, cache)?
                        .into_materialized();
                    let frame = apply_preserving_pixel(
                        compositor,
                        frame,
                        state,
                        &*modifier.resolve(state),
                    )?;
                    execution = ExecutionState::Materialized { frame, state };
                }
                PlannedOperation::Rasterize(drawing_strategy, sample_method) => {
                    execution.state_mut().skia_drawing_strategy = drawing_strategy;
                    execution.state_mut().sampling = sample_method;
                    execution = execution.materialize(compositor, canvas, cache_scope, cache)?;
                }
                PlannedOperation::MeshFlow {
                    grid_width,
                    grid_height,
                    source_offsets,
                } => {
                    let (frame, state) = execution
                        .materialize(compositor, canvas, cache_scope, cache)?
                        .into_materialized();
                    let frame = apply_mesh_flow(
                        compositor,
                        frame,
                        state,
                        grid_width,
                        grid_height,
                        &source_offsets,
                    )?;
                    execution = ExecutionState::Materialized { frame, state };
                }
                PlannedOperation::MotionBlur(transforms) => {
                    let (frame, state) = execution
                        .materialize(compositor, canvas, cache_scope, cache)?
                        .into_materialized();
                    let (frame, state) =
                        apply_motion_blur(compositor, canvas, frame, state, transforms)?;
                    execution = ExecutionState::Materialized { frame, state };
                }
                PlannedOperation::BeginAlphaMask(mask) => {
                    assert!(
                        alpha_mask_branch.is_none(),
                        "alpha mask branches cannot be nested"
                    );
                    let (frame, state) = execution
                        .materialize(compositor, canvas, cache_scope, cache)?
                        .into_materialized();
                    alpha_mask_branch = Some(AlphaMaskBranch {
                        original: clone_gpu_frame(&frame),
                        state,
                        mask,
                    });
                    execution = ExecutionState::Materialized { frame, state };
                }
                PlannedOperation::EndAlphaMask => {
                    let branch = alpha_mask_branch
                        .take()
                        .expect("alpha mask branch ended before it began");
                    let (affected, state) = execution
                        .materialize(compositor, canvas, cache_scope, cache)?
                        .into_materialized();
                    let (frame, state) =
                        apply_alpha_mask_branch(compositor, canvas, branch, affected, state)?;
                    execution = ExecutionState::Materialized { frame, state };
                }
            }
        }

        assert!(
            alpha_mask_branch.is_none(),
            "alpha mask branch was not ended"
        );

        let (frame, state) = execution
            .materialize(compositor, canvas, cache_scope, cache)?
            .into_materialized();
        Ok(frame_layer(frame, state))
    }
}

struct AlphaMaskBranch {
    original: GpuFrame,
    state: VisualState,
    mask: crate::alpha_mask::ResolvedShapeAlphaMask,
}

fn clone_gpu_frame(frame: &GpuFrame) -> GpuFrame {
    match frame {
        GpuFrame::Nv12(frame) => GpuFrame::Nv12(frame.clone()),
        GpuFrame::Rgba(frame) => GpuFrame::Rgba(frame.clone()),
    }
}

fn apply_alpha_mask_branch(
    compositor: &mut CudaVideoCompositor,
    canvas: CanvasSize,
    branch: AlphaMaskBranch,
    affected: GpuFrame,
    state: VisualState,
) -> Result<(GpuFrame, VisualState), String> {
    let source_size = match &branch.original {
        GpuFrame::Nv12(frame) => glam::Vec2::new(frame.width() as f32, frame.height() as f32),
        GpuFrame::Rgba(frame) => glam::Vec2::new(frame.width() as f32, frame.height() as f32),
    };
    let blend_mode = state.compositing.blend_mode;
    let opacity = branch.state.compositing.opacity;
    let plan = shrimply_video_core::alpha_mask::branch(
        branch.state.transform.matrix,
        opacity,
        state.compositing.opacity,
    )?;
    let render_branch = |compositor: &mut CudaVideoCompositor,
                         frame,
                         state: VisualState,
                         opacity: f32|
     -> Result<Rc<VisualFrame>, String> {
        compositor.render_layer_to_rgba(
            canvas,
            &frame_layer(
                frame,
                VisualState {
                    compositing: ResolvedCompositing {
                        opacity,
                        blend_mode: LayerBlendMode::Normal,
                    },
                    ..state
                },
            ),
        )
    };
    let original = render_branch(compositor, branch.original, branch.state, 1.0)?;
    let affected = render_branch(compositor, affected, state, plan.affected_opacity)?;
    let frame = crate::alpha_mask::combine_shape(
        compositor,
        &affected,
        original,
        plan.canvas_to_local,
        source_size,
        branch.mask,
    )?;
    Ok((
        GpuFrame::Rgba(Rc::new(frame)),
        VisualState {
            transform: ComposedTransform2D::IDENTITY,
            bounds: ResolvedVisualBounds::default(),
            sampling: state.sampling,
            skia_drawing_strategy: state.skia_drawing_strategy,
            compositing: ResolvedCompositing {
                opacity,
                blend_mode,
            },
        },
    ))
}

enum ExecutionState {
    Prepared {
        source: LazySource,
        state: VisualState,
        vector_operations: Vec<VectorOperation>,
    },
    Materialized {
        frame: GpuFrame,
        state: VisualState,
    },
}

impl ExecutionState {
    fn state_mut(&mut self) -> &mut VisualState {
        match self {
            Self::Prepared { state, .. } | Self::Materialized { state, .. } => state,
        }
    }

    fn materialize(
        self,
        compositor: &mut CudaVideoCompositor,
        _canvas: CanvasSize,
        _cache_scope: (&[Uuid], Uuid, Uuid),
        _cache: &mut VisualSourceCache,
    ) -> Result<Self, String> {
        match self {
            Self::Prepared {
                source,
                state,
                vector_operations,
            } => {
                let (frame, state) = match source {
                    LazySource::Data(data) => {
                        let layer = data.rasterize(
                            compositor,
                            state.skia_drawing_strategy,
                            &vector_operations,
                        )?;
                        (GpuFrame::Rgba(layer), state.baked())
                    }
                    LazySource::Frame(frame) => {
                        debug_assert!(vector_operations.is_empty());
                        (frame, state)
                    }
                };
                Ok(Self::Materialized { frame, state })
            }
            materialized @ Self::Materialized { .. } => Ok(materialized),
        }
    }

    fn into_materialized(self) -> (GpuFrame, VisualState) {
        match self {
            Self::Materialized { frame, state } => (frame, state),
            Self::Prepared { .. } => unreachable!("visual was not materialized"),
        }
    }
}

fn apply_pixel(
    compositor: &mut CudaVideoCompositor,
    canvas: CanvasSize,
    frame: GpuFrame,
    state: VisualState,
    modifier: &dyn GpuModifier,
) -> Result<(GpuFrame, VisualState), String> {
    if let GpuFrame::Rgba(layer) = &frame
        && !shrimply_render_core::effects::needs_canvas_materialization(
            (layer.width(), layer.height()),
            (canvas.width.max(1), canvas.height.max(1)),
            state.transform == ComposedTransform2D::IDENTITY
                && state.bounds == ResolvedVisualBounds::default(),
        )
    {
        return Ok((
            GpuFrame::Rgba(Rc::new(compositor.apply_rgba_modifier(layer, modifier)?)),
            state,
        ));
    }

    let compositing = state.compositing;
    let layer = frame_layer(
        frame,
        VisualState {
            compositing: ResolvedCompositing {
                opacity: 1.0,
                blend_mode: LayerBlendMode::Normal,
            },
            ..state
        },
    );
    let layer =
        compositor.render_layer_with_modifiers(canvas, &layer, std::slice::from_ref(&modifier))?;
    Ok((
        GpuFrame::Rgba(Rc::new(layer)),
        VisualState {
            transform: ComposedTransform2D::IDENTITY,
            bounds: ResolvedVisualBounds::default(),
            sampling: state.sampling,
            skia_drawing_strategy: state.skia_drawing_strategy,
            compositing,
        },
    ))
}

fn apply_preserving_pixel(
    compositor: &mut CudaVideoCompositor,
    frame: GpuFrame,
    state: VisualState,
    modifier: &dyn GpuModifier,
) -> Result<GpuFrame, String> {
    match frame {
        GpuFrame::Rgba(layer) => compositor
            .apply_rgba_modifier(&layer, modifier)
            .map(|layer| GpuFrame::Rgba(Rc::new(layer))),
        GpuFrame::Nv12(frame) => {
            let canvas = CanvasSize {
                width: frame.width(),
                height: frame.height(),
            };
            let layer = frame_layer(
                GpuFrame::Nv12(frame),
                VisualState {
                    transform: ComposedTransform2D::IDENTITY,
                    bounds: ResolvedVisualBounds::default(),
                    compositing: ResolvedCompositing {
                        opacity: 1.0,
                        blend_mode: LayerBlendMode::Normal,
                    },
                    ..state
                },
            );
            compositor
                .render_layer_with_modifiers(canvas, &layer, std::slice::from_ref(&modifier))
                .map_err(|error| {
                    format!(
                        "materialize NV12 input for {} modifier: {error}",
                        modifier.name()
                    )
                })
                .map(|layer| GpuFrame::Rgba(Rc::new(layer)))
        }
    }
}

fn apply_mesh_flow(
    compositor: &mut CudaVideoCompositor,
    frame: GpuFrame,
    state: VisualState,
    grid_width: u32,
    grid_height: u32,
    source_offsets: &[glam::Vec2],
) -> Result<GpuFrame, String> {
    let layer = match frame {
        GpuFrame::Rgba(layer) => layer,
        GpuFrame::Nv12(frame) => {
            let canvas = CanvasSize {
                width: frame.width(),
                height: frame.height(),
            };
            let layer = frame_layer(
                GpuFrame::Nv12(frame),
                VisualState {
                    transform: ComposedTransform2D::IDENTITY,
                    bounds: ResolvedVisualBounds::default(),
                    compositing: ResolvedCompositing {
                        opacity: 1.0,
                        blend_mode: LayerBlendMode::Normal,
                    },
                    ..state
                },
            );
            compositor.render_layer_to_rgba(canvas, &layer)?
        }
    };
    compositor
        .render_mesh_flow(&layer, grid_width, grid_height, source_offsets)
        .map(|layer| GpuFrame::Rgba(Rc::new(layer)))
}

fn apply_motion_blur(
    compositor: &mut CudaVideoCompositor,
    canvas: CanvasSize,
    frame: GpuFrame,
    state: VisualState,
    transforms: Rc<[ComposedTransform2D]>,
) -> Result<(GpuFrame, VisualState), String> {
    let compositing = state.compositing;
    let transforms: Rc<[ComposedTransform2D]> = transforms
        .iter()
        .map(|transform| transform.compose(state.transform))
        .collect::<Vec<_>>()
        .into();
    let mut layer = frame_layer(
        frame,
        VisualState {
            compositing: ResolvedCompositing {
                opacity: 1.0,
                blend_mode: LayerBlendMode::Normal,
            },
            ..state
        },
    );
    match &mut layer {
        VideoLayer::Nv12 { motion_blur, .. } | VideoLayer::Rgba { motion_blur, .. } => {
            *motion_blur = Some(transforms);
        }
    }
    let layer = compositor.render_layer_to_rgba(canvas, &layer)?;
    Ok((
        GpuFrame::Rgba(layer),
        VisualState {
            transform: ComposedTransform2D::IDENTITY,
            bounds: ResolvedVisualBounds::default(),
            compositing,
            ..state
        },
    ))
}

pub(crate) fn frame_layer(frame: GpuFrame, state: VisualState) -> VideoLayer {
    let source_size = match &frame {
        GpuFrame::Nv12(frame) => glam::Vec2::new(frame.width() as f32, frame.height() as f32),
        GpuFrame::Rgba(layer) => glam::Vec2::new(layer.width() as f32, layer.height() as f32),
    };
    let (crop, padding) = crate::visual_bounds::sampling_bounds(state.bounds, source_size);
    match frame {
        GpuFrame::Nv12(frame) => VideoLayer::Nv12 {
            frame,
            transform: state.transform,
            motion_blur: None,
            sample_method: state.sampling,
            compositing: state.compositing,
            crop,
            padding,
            address_mode: state.bounds.address_mode,
        },
        GpuFrame::Rgba(layer) => VideoLayer::Rgba {
            layer,
            transform: state.transform,
            motion_blur: None,
            sample_method: state.sampling,
            compositing: state.compositing,
            crop,
            padding,
            address_mode: state.bounds.address_mode,
        },
    }
}
