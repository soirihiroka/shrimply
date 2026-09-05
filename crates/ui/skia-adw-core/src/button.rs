use std::time::{Duration, Instant};

use skia_safe::Canvas;

use super::{Color, Rect, Vec2, draw_rounded_rect, math};

const TRANSITION_DURATION: Duration = Duration::from_millis(200);
const ADWAITA_RADIUS: f32 = 9.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Style {
    #[default]
    Raised,
    Flat,
    Pill,
    FlatPill,
    Circular,
    FlatCircular,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Radius {
    #[default]
    Style,
    Fixed(f32),
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Padding {
    #[default]
    Style,
    ImageButton,
    None,
    Uniform(f32),
    Symmetric {
        horizontal: f32,
        vertical: f32,
    },
}

#[derive(Clone, Copy)]
pub struct Config {
    bounds: Rect,
    color: Color,
    style: Style,
    radius: Radius,
    padding: Padding,
    checked: bool,
}

impl Config {
    pub fn new(bounds: Rect, color: Color) -> Self {
        Self {
            bounds,
            color,
            style: Style::default(),
            radius: Radius::default(),
            padding: Padding::default(),
            checked: false,
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn radius(mut self, radius: Radius) -> Self {
        self.radius = radius;
        self
    }

    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    PointerEntered,
    PointerLeft,
    Pressed,
    Released,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Response {
    pub handled: bool,
    pub clicked: bool,
    pub animating: bool,
}

#[derive(Clone, Copy)]
pub struct Draw {
    pub animating: bool,
    pub content_bounds: Rect,
}

#[derive(Clone, Copy, Default)]
pub struct Button {
    hovered: bool,
    pressed: bool,
    background: Option<f32>,
    target_background: f32,
    animation: Option<Animation>,
}

#[derive(Clone, Copy)]
struct Animation {
    source: f32,
    started_at: Instant,
}

impl Button {
    pub fn hit_test(bounds: Rect, point: Vec2) -> bool {
        bounds.contains(point)
    }

    pub fn event(&mut self, event: Event) -> Response {
        let old_hovered = self.hovered;
        let old_pressed = self.pressed;
        let mut response = Response::default();

        match event {
            Event::PointerEntered => self.hovered = true,
            Event::PointerLeft => self.hovered = false,
            Event::Pressed if self.hovered => {
                self.pressed = true;
                response.handled = true;
            }
            Event::Released => {
                response.handled = self.pressed;
                response.clicked = self.pressed && self.hovered;
                self.pressed = false;
            }
            Event::Cancelled => {
                self.hovered = false;
                self.pressed = false;
            }
            Event::Pressed => {}
        }

        response.animating = old_hovered != self.hovered || old_pressed != self.pressed;
        response
    }

    pub fn draw(&mut self, canvas: &Canvas, config: Config) -> Draw {
        let now = Instant::now();
        let target = background_alpha(config.style, config.checked, self.hovered, self.pressed);
        let current = self.background.unwrap_or(target);

        if self.background.is_none() {
            self.background = Some(target);
            self.target_background = target;
        } else if (target - self.target_background).abs() > f32::EPSILON {
            let current = animation_value(self.animation, current, self.target_background, now).0;
            self.background = Some(current);
            self.target_background = target;
            self.animation = Some(Animation {
                source: current,
                started_at: now,
            });
        }

        let (background, animating) = animation_value(
            self.animation,
            self.background.unwrap_or(target),
            self.target_background,
            now,
        );
        self.background = Some(background);
        if !animating {
            self.animation = None;
        }

        draw_rounded_rect(
            canvas,
            config.bounds,
            radius(config.style, config.radius, config.bounds),
            config.color.alpha_multiply(background),
        );

        Draw {
            animating,
            content_bounds: content_bounds(config.bounds, config.style, config.padding),
        }
    }
}

fn background_alpha(style: Style, checked: bool, hovered: bool, pressed: bool) -> f32 {
    let flat = matches!(style, Style::Flat | Style::FlatPill | Style::FlatCircular);
    match (flat, checked, hovered, pressed) {
        (true, false, _, true) => 0.16,
        (true, false, true, false) => 0.07,
        (true, false, false, false) => 0.0,
        (true, true, _, true) => 0.19,
        (true, true, true, false) => 0.13,
        (true, true, false, false) => 0.10,
        (false, false, _, true) => 0.30,
        (false, false, true, false) => 0.15,
        (false, false, false, false) => 0.10,
        (false, true, _, true) => 0.40,
        (false, true, true, false) => 0.35,
        (false, true, false, false) => 0.30,
    }
}

fn radius(style: Style, radius: Radius, bounds: Rect) -> f32 {
    match radius {
        Radius::Fixed(radius) => radius.max(0.0),
        Radius::Full => bounds.width().min(bounds.height()) / 2.0,
        Radius::Style
            if matches!(
                style,
                Style::Pill | Style::FlatPill | Style::Circular | Style::FlatCircular
            ) =>
        {
            bounds.width().min(bounds.height()) / 2.0
        }
        Radius::Style => ADWAITA_RADIUS,
    }
}

fn content_bounds(bounds: Rect, style: Style, padding: Padding) -> Rect {
    let (horizontal, vertical) = match padding {
        Padding::Style if matches!(style, Style::Pill | Style::FlatPill) => (32.0, 10.0),
        Padding::Style if matches!(style, Style::Circular | Style::FlatCircular) => (0.0, 0.0),
        Padding::Style => (10.0, 5.0),
        Padding::ImageButton => (5.0, 5.0),
        Padding::None => (0.0, 0.0),
        Padding::Uniform(padding) => (padding, padding),
        Padding::Symmetric {
            horizontal,
            vertical,
        } => (horizontal, vertical),
    };
    let horizontal = horizontal.max(0.0).min(bounds.width() / 2.0);
    let vertical = vertical.max(0.0).min(bounds.height() / 2.0);
    Rect::from_xywh(
        bounds.left() + horizontal,
        bounds.top() + vertical,
        (bounds.width() - horizontal * 2.0).max(0.0),
        (bounds.height() - vertical * 2.0).max(0.0),
    )
}

fn animation_value(
    animation: Option<Animation>,
    current: f32,
    target: f32,
    now: Instant,
) -> (f32, bool) {
    let Some(animation) = animation else {
        return (current, false);
    };
    let elapsed = now.duration_since(animation.started_at);
    if elapsed >= TRANSITION_DURATION {
        return (target, false);
    }

    let progress = elapsed.as_secs_f32() / TRANSITION_DURATION.as_secs_f32();
    let eased = math::cubic_bezier(progress, 0.25, 0.46, 0.45, 0.94);
    (animation.source + (target - animation.source) * eased, true)
}
