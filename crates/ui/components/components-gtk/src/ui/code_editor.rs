use std::{fs, path::Path, sync::OnceLock};

use sourceview5::prelude::*;

const CODE_EDITOR_HEIGHT: i32 = 180;
const INDENT_WIDTH: u32 = 4;
const SANIM_LANGUAGE: &str = include_str!("sanim.lang");
static SANIM_LANGUAGE_DIR: OnceLock<String> = OnceLock::new();

pub fn code_editor(
    value: &str,
    language: Option<&str>,
    changed: impl Fn(String) + 'static,
) -> gtk::Widget {
    let buffer = sourceview5::Buffer::new(None);
    super::multiline_text_input::set_text_style_scheme(&buffer);
    if let Some(language) = language {
        configure_code_language(&buffer, language);
    }
    buffer.set_highlight_matching_brackets(true);
    buffer.set_text(value);
    buffer.connect_changed(move |buffer| {
        let (start, end) = buffer.bounds();
        changed(buffer.text(&start, &end, true).to_string());
    });

    let view = sourceview5::View::with_buffer(&buffer);
    view.set_monospace(true);
    view.set_auto_indent(true);
    view.set_insert_spaces_instead_of_tabs(true);
    view.set_indent_on_tab(true);
    view.set_indent_width(i32::try_from(INDENT_WIDTH).unwrap_or(4));
    view.set_tab_width(INDENT_WIDTH);
    view.set_show_line_numbers(true);
    view.set_highlight_current_line(true);
    view.set_smart_backspace(true);
    view.set_smart_home_end(sourceview5::SmartHomeEndType::Before);
    view.set_accepts_tab(true);
    view.set_wrap_mode(gtk::WrapMode::None);

    gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(CODE_EDITOR_HEIGHT)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .build()
        .upcast()
}

pub fn configure_code_language(buffer: &sourceview5::Buffer, language: &str) {
    let manager = sourceview5::LanguageManager::new();
    if language == "sanim" {
        let path = sanim_language_dir();
        let mut paths: Vec<String> = manager
            .search_path()
            .into_iter()
            .map(|path| path.to_string())
            .collect();
        paths.push(path.to_string());
        manager.set_search_path(&paths.iter().map(String::as_str).collect::<Vec<_>>());
    }
    let language = manager
        .language(language)
        .unwrap_or_else(|| panic!("GtkSourceView language definition is unavailable: {language}"));
    buffer.set_language(Some(&language));
    buffer.set_highlight_syntax(true);
}

fn sanim_language_dir() -> &'static str {
    SANIM_LANGUAGE_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("shrimply-gtksourceview");
        write_sanim_language(&dir)
            .unwrap_or_else(|error| panic!("could not install Sanim syntax definition: {error}"));
        dir.into_os_string()
            .into_string()
            .expect("Sanim syntax definition path must be valid UTF-8")
    })
}

fn write_sanim_language(dir: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dir)?;
    let path = dir.join("sanim.lang");
    if fs::read_to_string(&path).ok().as_deref() != Some(SANIM_LANGUAGE) {
        fs::write(path, SANIM_LANGUAGE)?;
    }
    Ok(())
}
