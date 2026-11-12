use crate::TimelineStepVariant;
pub use shrimply_math_geometry::{ShapeKind, ShapeRoundingStrategy};

crate::timeline_step_type!(
    ShapeKind,
    ShapeKind::Rect,
    &[
        TimelineStepVariant {
            value: ShapeKind::Rect,
            key: "rect",
            label: "Rect",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Ellipse,
            key: "ellipse",
            label: "Ellipse",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Triangle,
            key: "triangle",
            label: "Triangle",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Star,
            key: "star",
            label: "Star",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Arrow,
            key: "arrow",
            label: "Arrow",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Diamond,
            key: "diamond",
            label: "Diamond",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Pentagon,
            key: "pentagon",
            label: "Pentagon",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Hexagon,
            key: "hexagon",
            label: "Hexagon",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Heart,
            key: "heart",
            label: "Heart",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Octagon,
            key: "octagon",
            label: "Octagon",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Cross,
            key: "cross",
            label: "Cross",
            icon: None
        },
    ]
);
crate::timeline_step_type!(
    ShapeRoundingStrategy,
    ShapeRoundingStrategy::Continuous,
    &[
        TimelineStepVariant {
            value: ShapeRoundingStrategy::Continuous,
            key: "continuous",
            label: "Continuous",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeRoundingStrategy::Circular,
            key: "circular",
            label: "Circular",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeRoundingStrategy::Chamfer,
            key: "chamfer",
            label: "Chamfer",
            icon: None
        },
    ]
);
