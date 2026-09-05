use std::time::Duration;

pub const FINAL_PREVIEW_DELAY: Duration = Duration::from_millis(450);
pub const LOCAL_SCRUB_WINDOW_SECONDS: i64 = 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Accuracy {
    #[default]
    BestEffort,
    Accurate,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompositeAccuracy {
    pub time: Accuracy,
    pub content: Accuracy,
    local_scrub: bool,
    continuous_playback: bool,
}

impl CompositeAccuracy {
    pub const BEST_EFFORT: Self = Self {
        time: Accuracy::BestEffort,
        content: Accuracy::BestEffort,
        local_scrub: false,
        continuous_playback: false,
    };
    pub const TIME_ACCURATE: Self = Self {
        time: Accuracy::Accurate,
        content: Accuracy::BestEffort,
        local_scrub: false,
        continuous_playback: false,
    };
    pub const CONTINUOUS_TIME_ACCURATE: Self = Self {
        time: Accuracy::Accurate,
        content: Accuracy::BestEffort,
        local_scrub: false,
        continuous_playback: true,
    };
    pub const LOCAL_TIME_ACCURATE: Self = Self {
        time: Accuracy::Accurate,
        content: Accuracy::BestEffort,
        local_scrub: true,
        continuous_playback: false,
    };
    pub const FULLY_ACCURATE: Self = Self {
        time: Accuracy::Accurate,
        content: Accuracy::Accurate,
        local_scrub: false,
        continuous_playback: false,
    };
    pub const LOCAL_FULLY_ACCURATE: Self = Self {
        time: Accuracy::Accurate,
        content: Accuracy::Accurate,
        local_scrub: true,
        continuous_playback: false,
    };

    pub const fn content_accurate(self) -> bool {
        matches!(self.content, Accuracy::Accurate)
    }

    pub const fn time_accurate(self) -> bool {
        matches!(self.time, Accuracy::Accurate)
    }

    pub const fn local_scrub(self) -> bool {
        self.local_scrub
    }

    pub const fn continuous_playback(self) -> bool {
        self.continuous_playback
    }
}
