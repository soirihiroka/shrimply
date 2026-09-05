use crate::generated::GeneratedTransition;
use shrimply_project::project::CanvasSize;
use skia_safe::{Canvas, ConditionallySend, FontMgr, Sendable, svg::Dom};
use std::sync::Mutex;

pub struct PreparedSvg {
    _source: String,
    dom: Mutex<Option<Sendable<Dom>>>,
}

impl PreparedSvg {
    pub fn new(source: String) -> Result<Self, String> {
        let dom = Dom::from_str(&source, FontMgr::new())
            .map_err(|error| format!("could not parse SVG: {error}"))?;
        let dom = dom
            .wrap_send()
            .map_err(|_| "parsed SVG was not uniquely owned".to_string())?;
        Ok(Self {
            _source: source,
            dom: Mutex::new(Some(dom)),
        })
    }

    fn with_dom<T>(&self, operation: impl FnOnce(&Dom) -> T) -> T {
        let mut stored = self.dom.lock().expect("parsed SVG mutex poisoned");
        let dom = stored
            .take()
            .expect("parsed SVG disappeared from its residency entry")
            .into_inner();
        let result = operation(&dom);
        *stored = Some(
            dom.wrap_send()
                .unwrap_or_else(|_| panic!("parsed SVG escaped while rendering")),
        );
        result
    }
}

impl PreparedSvg {
    pub fn draw(
        &self,
        canvas: &Canvas,
        root_size: CanvasSize,
        transition: Option<GeneratedTransition>,
        path_effect: Option<&skia_safe::PathEffect>,
    ) {
        self.with_dom(|dom| {
            let mut root = dom.root();
            root.set_width(skia_safe::svg::Length::new(
                root_size.width as f32,
                skia_safe::svg::LengthUnit::PX,
            ));
            root.set_height(skia_safe::svg::Length::new(
                root_size.height as f32,
                skia_safe::svg::LengthUnit::PX,
            ));

            let Some(transition) = transition else {
                if let Some(path_effect) = path_effect
                    && crate::svg_transition::draw_shaky(
                        dom,
                        &root,
                        canvas,
                        path_effect,
                        root_size.width as f32,
                        root_size.height as f32,
                    )
                {
                    return;
                }
                dom.render(canvas);
                return;
            };
            if path_effect.is_none()
                && ((transition.side == shrimply_project::project::TransitionSide::Intro
                    && transition.progress >= 1.0)
                    || (transition.side == shrimply_project::project::TransitionSide::Outro
                        && transition.progress <= 0.0))
            {
                dom.render(canvas);
                return;
            }
            if !crate::svg_transition::draw(
                dom,
                &root,
                canvas,
                transition,
                root_size.width as f32,
                root_size.height as f32,
                path_effect,
            ) {
                dom.render(canvas);
            }
        });
    }
}

impl PreparedSvg {
    pub fn morph_scene(
        &self,
        root_size: CanvasSize,
        canvas_size: CanvasSize,
        evaluation: &shrimply_evaluation::VisualEvaluation,
    ) -> Option<crate::vector_morph::MorphScene> {
        self.with_dom(|dom| {
            let mut root = dom.root();
            root.set_width(skia_safe::svg::Length::new(
                root_size.width as f32,
                skia_safe::svg::LengthUnit::PX,
            ));
            root.set_height(skia_safe::svg::Length::new(
                root_size.height as f32,
                skia_safe::svg::LengthUnit::PX,
            ));
            let objects = crate::svg_transition::svg_paths(
                &root,
                root_size.width as f32,
                root_size.height as f32,
            )
            .into_iter()
            .map(|path| {
                let mut appearance = Vec::new();
                if path.fill {
                    let mut paint = skia_safe::Paint::default();
                    paint.set_anti_alias(true);
                    paint.set_color(path.fill_color);
                    paint.set_alpha_f(paint.alpha_f() * path.fill_opacity);
                    appearance.push(crate::vector_morph::MorphPaintLayer {
                        paint,
                        offset: glam::Vec2::ZERO,
                    });
                }
                if path.stroke_width > 0.0 {
                    let mut paint = skia_safe::Paint::default();
                    paint.set_anti_alias(true);
                    paint.set_style(skia_safe::PaintStyle::Stroke);
                    paint.set_stroke_width(path.stroke_width);
                    paint.set_color(path.stroke_color);
                    paint.set_alpha_f(paint.alpha_f() * path.stroke_opacity);
                    appearance.push(crate::vector_morph::MorphPaintLayer {
                        paint,
                        offset: glam::Vec2::ZERO,
                    });
                }
                crate::vector_morph::MorphObject {
                    path: crate::vector_morph::skia_path_to_morph(&path.path),
                    appearance,
                }
            })
            .collect();
            Some(crate::vector_morph::MorphScene {
                objects,
                evaluation: evaluation.clone(),
                canvas_size,
            })
        })
    }
}
