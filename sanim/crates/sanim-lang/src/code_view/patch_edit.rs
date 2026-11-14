//! Apply every range edit through the same source/provenance and patch validator.
use super::*;

pub(super) fn apply(
    original: &sanim_core::code::Document,
    args: &imbl::OrdMap<String, Value>,
    owner: &str,
) -> Result<sanim_core::code::Document, String> {
    let selection = args.get("range").map(range).transpose()?;
    let bounds = selection
        .as_ref()
        .map(|selection| {
            selection.validate_for(owner, original)?;
            selection.byte_range(&original.text)
        })
        .transpose()?;
    let target = args.get("to").map(document).transpose()?;
    let replacement = args.get("code").map(Value::string).transpose()?;
    if target.is_some() == replacement.is_some() {
        return Err(
            "CodeRange.edit requires exactly one of code=String or src=\"source-or-patch\"".into(),
        );
    }
    if let Some(target) = &target
        && matches!(target.content, sanim_core::code::Content::Patch(_))
    {
        let changed = original.apply(target, args.get("file").map(Value::string).transpose()?)?;
        if let Some(bounds) = bounds {
            // Context lines may extend outside the selection; only actual edits
            // are fenced. Validate the entire patch before testing this contract.
            let prefix = &original.text[..bounds.start];
            let suffix = &original.text[bounds.end..];
            if changed.text.len() < prefix.len() + suffix.len()
                || !changed.text.starts_with(prefix)
                || !changed.text.ends_with(suffix)
            {
                return Err("patch changes text outside the selected CodeRange; select the affected range or apply the patch to the whole CodeView".into());
            }
        }
        return Ok(changed);
    }
    let replacement = replacement
        .or_else(|| target.as_ref().map(|target| target.text.as_ref()))
        .expect("validated edit source");
    let text = if let Some(bounds) = bounds {
        let bytes = original.text.len() - (bounds.end - bounds.start);
        let bytes = bytes
            .checked_add(replacement.len())
            .ok_or("edited code length overflow")?;
        if bytes > sanim_core::code::MAX_CODE_BYTES {
            return Err("edited source exceeds the code byte budget".into());
        }
        let mut text = String::with_capacity(bytes);
        text.push_str(&original.text[..bounds.start]);
        text.push_str(replacement);
        text.push_str(&original.text[bounds.end..]);
        text
    } else {
        replacement.to_owned()
    };
    sanim_core::code::Document::source_shared(&original.path, text.into(), original.language)
}
