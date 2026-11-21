//! One font browser protocol shared by preferences and inspector font lists.
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_inspector_core::{font_cache, font_selector::Browser};
use shrimply_project_document::FontFamily;
use std::{collections::VecDeque, path::PathBuf};

#[derive(Default)]
pub(crate) struct Picker {
    fonts: Browser,
    open: bool,
    serial: u64,
    activation: Option<u64>,
    preview_revision: u64,
    preview_sources: Vec<(PathBuf, usize)>,
    previews: VecDeque<(u64, Vec<font_cache::NativeFontPreview>)>,
}

impl Picker {
    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn poll(&mut self) -> (bool, Vec<FontFamily>) {
        let poll = self.fonts.poll();
        let mut activated = Vec::new();
        for result in poll.activations {
            let owner = self.activation.take();
            if self.open
                && owner == Some(self.serial)
                && let Ok(family) = result
            {
                activated.push(font_cache::project_family(&family));
                self.open = false;
            }
        }
        (poll.changed, activated)
    }

    pub fn snapshot(&mut self, selected: &[FontFamily], operation: &str) -> Option<Value> {
        if !self.open {
            return None;
        }
        let mut previews = Vec::new();
        let command = |action: &str| json!({"op":operation,"action":action,"picker":self.serial});
        let choices = self.fonts.visible().iter().map(|font| {
            let family = font_cache::project_family(font);
            let preview = font_cache::native_preview_source(font);
            let mut choose = command("font_choose");
            choose["name"] = json!(font.name);
            choose["google"] = json!(font.source == font_cache::FontSource::Google);
            choose["revision"] = json!(font.revision);
            let row = json!({"label":font.name,"source":text(match font.source {font_cache::FontSource::Local=>"Local",font_cache::FontSource::Google=>"Google Fonts"}),
                "selected":selected.contains(&family),"preview_file":preview.as_ref().map(|preview|&preview.path),"preview_index":preview.as_ref().map_or(0,|preview|preview.index),"command":choose});
            if let Some(preview) = preview {previews.push(preview);}
            row
        }).collect::<Vec<_>>();
        let sources = previews
            .iter()
            .map(|preview| (preview.path.clone(), preview.index))
            .collect::<Vec<_>>();
        if sources != self.preview_sources {
            self.preview_revision = self
                .preview_revision
                .checked_add(1)
                .expect("font preview revision overflow");
            self.preview_sources = sources;
            self.previews.push_back((self.preview_revision, previews));
        }
        let mut visible = command("font_visible");
        visible["preview_revision"] = json!(self.preview_revision);
        Some(
            json!({"id":format!("{operation}:font:{}",self.serial),"title":text("Choose font"),"search_label":text("Search"),
            "search_delay_ms":shrimply_inspector_core::font_selector::GOOGLE_LOOKUP_DELAY.as_millis(),"busy":self.fonts.busy(),"status":text(self.fonts.status()),
            "close":command("font_close"),"close_label":text("Close"),"search":command("font_search"),"preview_revision":self.preview_revision,"visible":visible,"choices":choices}),
        )
    }

    pub fn apply(&mut self, request: &Value) -> Result<bool, String> {
        let action = request["action"]
            .as_str()
            .ok_or("Font action is required")?;
        if action != "font_open" && (!self.open || request["picker"].as_u64() != Some(self.serial))
        {
            if matches!(action, "font_search" | "font_visible" | "font_close") {
                return Ok(self.open);
            }
            return Err("The font picker changed; open it again".into());
        }
        match action {
            "font_open" => {
                if self.open {
                    return Ok(self.open);
                }
                self.previews.clear();
                self.preview_sources.clear();
                self.preview_revision = 0;
                self.fonts.open();
                self.fonts.search("");
                self.open = true;
                self.serial = self
                    .serial
                    .checked_add(1)
                    .expect("font picker serial overflow");
            }
            "font_close" => self.close(),
            "font_search" => {
                self.fonts.search(
                    request["query"]
                        .as_str()
                        .ok_or("Font search text is required")?,
                );
            }
            "font_visible" => {
                let adopted = request["preview_revision"]
                    .as_u64()
                    .filter(|revision| *revision <= self.preview_revision)
                    .ok_or("The font preview document changed")?;
                self.previews.retain(|(revision, _)| *revision >= adopted);
                let start = request["start"]
                    .as_u64()
                    .ok_or("Font viewport start is required")? as usize;
                let end = request["end"]
                    .as_u64()
                    .ok_or("Font viewport end is required")? as usize;
                self.fonts.request_previews(
                    start..end,
                    shrimply_inspector_core::font_selector::PreviewFormat::FontFile,
                );
            }
            "font_choose" => {
                let family = self
                    .fonts
                    .visible()
                    .iter()
                    .find(|family| {
                        request["name"] == family.name
                            && request["google"]
                                == (family.source == font_cache::FontSource::Google)
                            && request["revision"] == family.revision
                    })
                    .cloned()
                    .ok_or("The font choice changed; search again")?;
                self.fonts.activate(family)?;
                self.activation = Some(self.serial);
            }
            _ => return Err("Unknown font picker action".into()),
        }
        Ok(self.open)
    }
}
