//! Font-family list edits use the same picker, activation and family rules as GTK.
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorController, InspectorTarget, font_selector::FamilyEdit,
};
use shrimply_project_document::FontFamily;

use crate::{InspectorDocument, font_picker::Picker};

#[derive(Default)]
pub struct Fonts {
    picker: Picker,
    owner: Option<Owner>,
}
struct Owner {
    target: InspectorTarget,
    control: Value,
    families: Vec<FontFamily>,
    index: Option<usize>,
}

pub(super) fn presentation(control: &InspectorControl, item: &str, id: &str) -> Value {
    let families: Vec<FontFamily> =
        serde_json::from_str(&control.value).expect("font families must be valid");
    let command = |action: &str, index: Option<usize>| json!({"op":"inspector_fonts","item":item,"control":id,"version":control.value,"action":action,"index":index});
    let rows = families.iter().enumerate().map(|(index,family)|json!({"label":family.name(),"subtitle":text(match family {FontFamily::Local{..}=>"Local",FontFamily::GoogleFonts{..}=>"Google Fonts"}),"command":command("font_open",Some(index)),
        "actions":[
            {"label":text("Move font up"),"icon":"go-up-symbolic","enabled":index>0,"command":command("move-up",Some(index))},
            {"label":text("Move font down"),"icon":"go-down-symbolic","enabled":index+1<families.len(),"command":command("move-down",Some(index))},
            {"label":text("Remove font"),"icon":"user-trash-symbolic","enabled":true,"command":command("remove",Some(index))}]
    })).collect::<Vec<_>>();
    json!({"id":id,"widget":"action_list","label":text(&control.label),"subtitle":text(&control.subtitle),"enabled":control.sensitive,"editable":control.editable&&control.sensitive,
        "empty":text("System default"),"rows":rows,"add":{"label":text("Add font…"),"command":command("font_open",None)}})
}

impl Fonts {
    fn synchronize(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
    ) -> bool {
        let valid = self.owner.as_ref().is_none_or(|owner| {
            controller.target() == owner.target
                && document.target == owner.target
                && super::resolve_control(document, &owner.control).is_ok_and(|control| {
                    controller
                        .control_font_families(&owner.target, control)
                        .is_ok_and(|families| families == owner.families)
                })
        });
        if !valid {
            self.picker.close();
            self.owner = None;
        }
        !valid
    }

    pub fn poll(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
    ) -> Result<bool, String> {
        let invalid = self.synchronize(controller, document);
        let (changed, activated) = self.picker.poll();
        for family in activated {
            let owner = self.owner.take().ok_or("Font picker target changed")?;
            let control = super::resolve_control(document, &owner.control)?;
            controller.edit_control_font_family(
                &owner.target,
                control,
                match owner.index {
                    Some(index) => FamilyEdit::Replace { index, family },
                    None => FamilyEdit::Append(family),
                },
            )?;
        }
        Ok(invalid || changed)
    }

    pub fn decorate(&mut self, document: &InspectorDocument, snapshot: &mut Value) {
        if let Some(owner) = &self.owner
            && document.target == owner.target
        {
            let selected = owner
                .index
                .and_then(|index| owner.families.get(index))
                .cloned()
                .into_iter()
                .collect::<Vec<_>>();
            if let Some(picker) = self.picker.snapshot(&selected, "inspector_fonts") {
                snapshot["font_picker"] = picker;
            }
        }
    }

    pub fn apply(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
        request: &Value,
    ) -> Result<(), String> {
        if controller.target() != document.target {
            return Err("Inspector selection changed".into());
        }
        self.synchronize(controller, document);
        match request["action"]
            .as_str()
            .ok_or("Font action is required")?
        {
            "font_open" | "move-up" | "move-down" | "remove" => {
                let control = super::resolve_control(document, request)?;
                if control.kind != ControlKind::FontFamilies
                    || !control.editable
                    || request["version"] != control.value
                {
                    return Err("Inspector font list changed".into());
                }
                let families: Vec<FontFamily> =
                    serde_json::from_str(&control.value).map_err(|error| error.to_string())?;
                if families != controller.control_font_families(&document.target, control)? {
                    return Err("Inspector font list changed".into());
                }
                let index = if request["index"].is_null() {
                    None
                } else {
                    Some(
                        request["index"]
                            .as_u64()
                            .and_then(|index| usize::try_from(index).ok())
                            .filter(|index| *index < families.len())
                            .ok_or("Font row is unavailable")?,
                    )
                };
                let action = request["action"].as_str().expect("validated font action");
                if action == "font_open" {
                    self.picker.close();
                    self.picker.apply(&json!({"action":"font_open"}))?;
                    self.owner = Some(Owner {
                        target: document.target.clone(),
                        control: request.clone(),
                        families,
                        index,
                    });
                } else {
                    let index = index.ok_or("Font row is required")?;
                    let edit = match action {
                        "move-up" => FamilyEdit::Move { index, offset: -1 },
                        "move-down" => FamilyEdit::Move { index, offset: 1 },
                        _ => FamilyEdit::Remove(index),
                    };
                    controller.edit_control_font_family(&document.target, control, edit)?;
                }
            }
            _ => {
                if !self.picker.apply(request)? {
                    self.owner = None;
                }
            }
        }
        Ok(())
    }
}
