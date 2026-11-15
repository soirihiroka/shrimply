use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_preferences_core::{PreferencePage, remote};
use shrimply_trust_core::{Entry, Kind, Review};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Sources {
    pending: Option<(String, Kind)>,
    dialog: Option<Value>,
}

impl Sources {
    pub fn decorate(&self, document: &mut Value) {
        document["source_picker"]=self.pending.as_ref().map(|(id,kind)|json!({"id":id,"folder":*kind==Kind::Folder,"title":text("Add trusted source"),
            "complete":{"action":"trust_add","picker":id},"cancel":{"action":"trust_cancel","picker":id}})).unwrap_or(Value::Null);
        document["dialog"] = self.dialog.clone().unwrap_or(Value::Null);
    }

    pub fn apply(&mut self, request: &Value) -> Result<Option<PathBuf>, String> {
        match request["action"]
            .as_str()
            .ok_or("Trust action is required")?
        {
            "trust_pick" => self.pending = Some((uuid::Uuid::new_v4().to_string(), kind(request)?)),
            "trust_cancel" | "trust_add" => {
                let (id, kind) = self.pending.as_ref().ok_or("The source picker is closed")?;
                if request["picker"] != *id {
                    return Err("The source picker changed".into());
                }
                let kind = *kind;
                self.pending = None;
                if request["action"] == "trust_add" {
                    let path=request["path"].as_str().ok_or_else(||text("Open the source project before trusting files in its managed copy.").into_owned())?;
                    let path = PathBuf::from(path)
                        .canonicalize()
                        .map_err(|error| error.to_string())?;
                    match kind {
                        Kind::File => Review {
                            files: vec![path],
                            folders: Vec::new(),
                        },
                        Kind::Folder => Review {
                            files: Vec::new(),
                            folders: vec![path],
                        },
                    }
                    .approve(kind)?;
                }
            }
            action @ ("trust_remove" | "trust_info" | "trust_show") => {
                let kind = kind(request)?;
                let entry = shrimply_trust_core::entries()?
                    .into_iter()
                    .find(|entry| {
                        request["path"] == entry.path.to_string_lossy().as_ref()
                            && entry.kind == kind
                    })
                    .ok_or("The trusted source changed")?;
                match action {
                    "trust_remove" => shrimply_trust_core::remove(&entry)?,
                    "trust_info" => {
                        let presentation = entry.presentation();
                        self.dialog = Some(
                            json!({"title":presentation.title,"body":presentation.body,"close_label":presentation.close_label,"command":{"action":"dialog_close"}}),
                        );
                    }
                    "trust_show" => return Ok(Some(entry.path)),
                    _ => unreachable!("trusted source action"),
                }
            }
            "dialog_close" => self.dialog = None,
            _ => return Err("Unknown trust action".into()),
        }
        Ok(None)
    }

    pub fn page(&self) -> Value {
        let mut controls=match shrimply_trust_core::entries() {
            Ok(entries)=>entries.into_iter().map(|Entry{path,kind}| {
                let kind=match kind {Kind::File=>"file",Kind::Folder=>"folder"};
                json!({"widget":"choice","choice_style":"row","label":path.to_string_lossy(),"display":text(if kind=="file" {"File"}else{"Folder"}),"choices":[
                    {"label":text("Info"),"command":{"action":"trust_info","path":path,"kind":kind}},
                    {"label":text("Show in File Manager"),"command":{"action":"trust_show","path":path,"kind":kind}},
                    {"label":text("Remove"),"command":{"action":"trust_remove","path":path,"kind":kind}}
                ]})
            }).collect::<Vec<_>>(),
            Err(error)=>vec![json!({"widget":"text","label":text("Could not read trusted locations"),"display":error})],
        };
        controls.push(json!({"widget":"choice","choice_style":"row","label":text("Add source"),"display":"","choice_description":text(shrimply_trust_core::EXPLANATION),"choices":[
            {"label":text("Choose File"),"command":{"action":"trust_pick","kind":"file"}},
            {"label":text("Choose Folder"),"command":{"action":"trust_pick","kind":"folder"}}
        ]}));
        remote::page(
            PreferencePage::Trust,
            vec![json!({"widget":"group","label":text("Trusted sources"),"controls":controls})],
        )
    }
}

fn kind(request: &Value) -> Result<Kind, String> {
    match request["kind"].as_str() {
        Some("file") => Ok(Kind::File),
        Some("folder") => Ok(Kind::Folder),
        _ => Err("A file or folder source is required".into()),
    }
}
