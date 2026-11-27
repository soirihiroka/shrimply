use shrimply_components_gtk::tr;
use std::{cell::RefCell, rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::glib::{self, SourceId};
use shrimply_math_color::Color;
use sourceview5::prelude::*;

pub(crate) use shrimply_inspector_core::rhai_editor::ExpressionValue;
use shrimply_inspector_core::rhai_editor::{
    self, DIAGNOSTIC_DEBOUNCE_MILLISECONDS, DiagnosticCache, EDITOR_HEIGHT, INDENT_WIDTH,
};

const INDENT: &str = "  ";
const ERROR_COLOR: Color = Color::new(1.0, 0.45, 0.42, 1.0);

pub(crate) fn editor(
    source: Option<String>,
    value: ExpressionValue,
    update: impl Fn(String) + 'static,
) -> gtk::Widget {
    let buffer = sourceview5::Buffer::new(None);
    set_rhai_language(&buffer);
    set_style_scheme(&buffer);
    buffer.set_text(source.as_deref().unwrap_or_default());

    let error_color = ERROR_COLOR.into();
    let diagnostic_tag = gtk::TextTag::builder()
        .name("rhai-error")
        .foreground_rgba(&error_color)
        .underline(gtk::pango::Underline::Error)
        .underline_rgba(&error_color)
        .build();
    let tag_table = buffer.tag_table();
    tag_table.add(&diagnostic_tag);
    diagnostic_tag.set_priority(tag_table.size() - 1);

    let views = Rc::new(RefCell::new(Vec::new()));
    let diagnostic_message = Rc::new(RefCell::new(None));
    let diagnostic_labels = Rc::new(RefCell::new(Vec::new()));
    let inline_view = editor_view(&buffer, false);
    connect_editor_keys(&inline_view, &buffer, value);
    track_view(&views, &inline_view);

    let (words, seed_words) = completion_words(value);
    words.register(&buffer);
    inline_view.completion().add_provider(&words);

    let pending_diagnostic = Rc::new(RefCell::new(None));
    let diagnostic_cache = Rc::new(RefCell::new(DiagnosticCache::default()));
    update_diagnostic(
        &buffer,
        &diagnostic_tag,
        &diagnostic_message,
        &diagnostic_labels,
        &diagnostic_cache,
    );

    let update = Rc::new(update);
    let changed_pending = pending_diagnostic.clone();
    let changed_tag = diagnostic_tag.clone();
    let changed_views = views.clone();
    let changed_diagnostic_message = diagnostic_message.clone();
    let changed_diagnostic_labels = diagnostic_labels.clone();
    let changed_diagnostic_cache = diagnostic_cache.clone();
    buffer.connect_changed(move |buffer| {
        let (start, end) = buffer.bounds();
        update(buffer.text(&start, &end, true).to_string());
        trigger_completion(buffer, &changed_views, value);
        schedule_diagnostic(
            buffer,
            &changed_tag,
            &changed_diagnostic_message,
            &changed_diagnostic_labels,
            &changed_pending,
            &changed_diagnostic_cache,
        );
    });

    let scroller = gtk::ScrolledWindow::builder()
        .child(&inline_view)
        .min_content_height(EDITOR_HEIGHT)
        .hexpand(true)
        .build();

    let expand = gtk::Button::builder()
        .icon_name("view-fullscreen-symbolic")
        .tooltip_text(tr!("Open larger editor").as_ref())
        .valign(gtk::Align::Start)
        .build();
    expand.add_css_class("flat");
    let dialog_buffer = buffer.clone();
    let dialog_words = words.clone();
    let dialog_seed_words = seed_words.clone();
    let dialog_views = views.clone();
    let dialog_diagnostic_message = diagnostic_message.clone();
    let dialog_diagnostic_labels = diagnostic_labels.clone();
    expand.connect_clicked(move |button| {
        let _seed_words = dialog_seed_words.clone();
        open_large_editor(
            button,
            &dialog_buffer,
            dialog_words.clone(),
            dialog_views.clone(),
            dialog_diagnostic_message.clone(),
            dialog_diagnostic_labels.clone(),
            value,
        );
    });

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&scroller);
    row.append(&expand);
    let cleanup_words = words.clone();
    let cleanup_buffer = buffer.clone();
    let cleanup_seed_words = seed_words.clone();
    row.connect_destroy(move |_| {
        if let Some(source_id) = pending_diagnostic.borrow_mut().take() {
            source_id.remove();
        }
        cleanup_words.unregister(&cleanup_buffer);
        cleanup_words.unregister(&cleanup_seed_words);
    });
    row.upcast()
}

fn editor_view(buffer: &sourceview5::Buffer, large: bool) -> sourceview5::View {
    let view = sourceview5::View::with_buffer(buffer);
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(|_| shrimply_project_document::project::finish_coalesced_edit());
    view.add_controller(focus);
    buffer.set_highlight_matching_brackets(true);
    view.set_monospace(true);
    view.set_auto_indent(true);
    view.set_insert_spaces_instead_of_tabs(true);
    view.set_indent_on_tab(true);
    view.set_indent_width(INDENT_WIDTH as i32);
    view.set_smart_backspace(true);
    view.set_smart_home_end(sourceview5::SmartHomeEndType::Before);
    view.set_show_line_numbers(large);
    view.set_highlight_current_line(large);
    view.set_accepts_tab(true);
    view.set_tab_width(INDENT_WIDTH as u32);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    if !large {
        view.set_height_request(EDITOR_HEIGHT);
    }
    view
}

fn connect_editor_keys(
    view: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    value: ExpressionValue,
) {
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let buffer = buffer.clone();
    key_controller.connect_key_pressed(move |_, key, _, state| {
        let command = state.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        );

        if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) && key == gtk::gdk::Key::slash {
            toggle_line_comments(&buffer);
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Tab
            && !state.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK,
            )
            && complete_current_prefix(&buffer, value)
        {
            return glib::Propagation::Stop;
        }
        if command {
            return glib::Propagation::Proceed;
        }

        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
            insert_indented_newline(&buffer);
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::BackSpace && delete_empty_pair(&buffer) {
            return glib::Propagation::Stop;
        }
        if let Some(open) = key
            .to_unicode()
            .filter(|character| is_opening_pair(*character))
        {
            if wrap_or_insert_pair(&buffer, open) {
                return glib::Propagation::Stop;
            }
        } else if let Some(close) = key
            .to_unicode()
            .filter(|character| is_closing_pair(*character))
            && skip_or_insert_closer(&buffer, close)
        {
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    view.add_controller(key_controller);
}

fn is_opening_pair(character: char) -> bool {
    matches!(character, '(' | '[' | '{' | '"' | '\'' | '`')
}

fn is_closing_pair(character: char) -> bool {
    matches!(character, ')' | ']' | '}')
}

fn closing_pair(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '"' | '\'' | '`' => open,
        _ => unreachable!("not an opening delimiter"),
    }
}

fn wrap_or_insert_pair(buffer: &sourceview5::Buffer, open: char) -> bool {
    let close = closing_pair(open);
    if let Some((start, end)) = buffer.selection_bounds() {
        let start_offset = start.offset();
        let end_offset = end.offset();
        buffer.begin_user_action();
        let mut end = end;
        buffer.insert(&mut end, &close.to_string());
        let mut start = buffer.iter_at_offset(start_offset);
        buffer.insert(&mut start, &open.to_string());
        buffer.select_range(
            &buffer.iter_at_offset(start_offset + 1),
            &buffer.iter_at_offset(end_offset + 1),
        );
        buffer.end_user_action();
        return true;
    }

    let cursor = buffer.cursor_position();
    if matches!(open, '"' | '\'' | '`') {
        if is_escaped(buffer, cursor) {
            return false;
        }
        if character_at(buffer, cursor) == Some(open) {
            buffer.place_cursor(&buffer.iter_at_offset(cursor + 1));
            return true;
        }
    }

    buffer.begin_user_action();
    buffer.insert_at_cursor(&format!("{open}{close}"));
    buffer.place_cursor(&buffer.iter_at_offset(cursor + 1));
    buffer.end_user_action();
    true
}

fn skip_or_insert_closer(buffer: &sourceview5::Buffer, close: char) -> bool {
    if buffer.selection_bounds().is_some() {
        return false;
    }
    let cursor = buffer.cursor_position();
    if character_at(buffer, cursor) == Some(close) {
        buffer.place_cursor(&buffer.iter_at_offset(cursor + 1));
        return true;
    }

    let cursor_iter = buffer.iter_at_offset(cursor);
    let mut line_start = cursor_iter;
    line_start.set_line_offset(0);
    let prefix = buffer.text(&line_start, &cursor_iter, true);
    if !prefix
        .chars()
        .all(|character| matches!(character, ' ' | '\t'))
    {
        return false;
    }

    let remove = prefix
        .chars()
        .rev()
        .take(INDENT_WIDTH)
        .take_while(|character| *character == ' ')
        .count() as i32;
    let remove = if remove == 0 && prefix.ends_with('\t') {
        1
    } else {
        remove
    };
    if remove == 0 {
        return false;
    }

    buffer.begin_user_action();
    let mut start = buffer.iter_at_offset(cursor - remove);
    let mut end = cursor_iter;
    buffer.delete(&mut start, &mut end);
    buffer.insert_at_cursor(&close.to_string());
    buffer.end_user_action();
    true
}

fn delete_empty_pair(buffer: &sourceview5::Buffer) -> bool {
    if buffer.selection_bounds().is_some() {
        return false;
    }
    let cursor = buffer.cursor_position();
    if cursor == 0 {
        return false;
    }
    let Some(open) = character_at(buffer, cursor - 1) else {
        return false;
    };
    if !is_opening_pair(open) || character_at(buffer, cursor) != Some(closing_pair(open)) {
        return false;
    }

    buffer.begin_user_action();
    let mut start = buffer.iter_at_offset(cursor - 1);
    let mut end = buffer.iter_at_offset(cursor + 1);
    buffer.delete(&mut start, &mut end);
    buffer.place_cursor(&buffer.iter_at_offset(cursor - 1));
    buffer.end_user_action();
    true
}

fn insert_indented_newline(buffer: &sourceview5::Buffer) {
    buffer.begin_user_action();
    if let Some((mut start, mut end)) = buffer.selection_bounds() {
        buffer.delete(&mut start, &mut end);
    }

    let cursor = buffer.cursor_position();
    let cursor_iter = buffer.iter_at_offset(cursor);
    let mut line_start = cursor_iter;
    line_start.set_line_offset(0);
    let prefix = buffer.text(&line_start, &cursor_iter, true);
    let base_indent: String = prefix
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect();
    let open = prefix
        .trim_end()
        .chars()
        .next_back()
        .filter(|character| matches!(character, '(' | '[' | '{'));
    let indent = if open.is_some() {
        format!("{base_indent}{INDENT}")
    } else {
        base_indent.clone()
    };

    if open.is_some_and(|open| character_at(buffer, cursor) == Some(closing_pair(open))) {
        buffer.insert_at_cursor(&format!("\n{indent}\n{base_indent}"));
        buffer.place_cursor(&buffer.iter_at_offset(cursor + 1 + indent.chars().count() as i32));
    } else {
        buffer.insert_at_cursor(&format!("\n{indent}"));
    }
    buffer.end_user_action();
}

fn toggle_line_comments(buffer: &sourceview5::Buffer) {
    let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
        let cursor = buffer.iter_at_offset(buffer.cursor_position());
        (cursor, cursor)
    });
    let first_line = start.line();
    let last_line = if end.line_offset() == 0 && end.line() > first_line {
        end.line() - 1
    } else {
        end.line()
    };
    let lines: Vec<i32> = (first_line..=last_line)
        .filter(|line| {
            buffer.iter_at_line(*line).is_some_and(|start| {
                let mut end = start;
                end.forward_to_line_end();
                !buffer.text(&start, &end, true).trim().is_empty()
            })
        })
        .collect();
    if lines.is_empty() {
        return;
    }
    let uncomment = lines.iter().all(|line| {
        let start = buffer.iter_at_line(*line).expect("line disappeared");
        let mut end = start;
        end.forward_to_line_end();
        buffer
            .text(&start, &end, true)
            .trim_start()
            .starts_with("//")
    });

    buffer.begin_user_action();
    for line in lines.into_iter().rev() {
        let line_start = buffer.iter_at_line(line).expect("line disappeared");
        let mut line_end = line_start;
        line_end.forward_to_line_end();
        let text = buffer.text(&line_start, &line_end, true);
        let indent = text
            .chars()
            .take_while(|character| matches!(character, ' ' | '\t'))
            .count() as i32;
        let offset = line_start.offset() + indent;
        if uncomment {
            let remove = if text[indent as usize..].starts_with("// ") {
                3
            } else {
                2
            };
            let mut start = buffer.iter_at_offset(offset);
            let mut end = buffer.iter_at_offset(offset + remove);
            buffer.delete(&mut start, &mut end);
        } else {
            let mut insert = buffer.iter_at_offset(offset);
            buffer.insert(&mut insert, "// ");
        }
    }
    buffer.end_user_action();
}

fn is_escaped(buffer: &sourceview5::Buffer, offset: i32) -> bool {
    let mut offset = offset;
    let mut backslashes = 0;
    while offset > 0 && character_at(buffer, offset - 1) == Some('\\') {
        backslashes += 1;
        offset -= 1;
    }
    backslashes % 2 == 1
}

fn character_at(buffer: &sourceview5::Buffer, offset: i32) -> Option<char> {
    let iter = buffer.iter_at_offset(offset);
    (!iter.is_end()).then(|| iter.char())
}

fn open_large_editor(
    parent: &gtk::Button,
    buffer: &sourceview5::Buffer,
    words: sourceview5::CompletionWords,
    views: Rc<RefCell<Vec<glib::WeakRef<sourceview5::View>>>>,
    diagnostic_message: Rc<RefCell<Option<String>>>,
    diagnostic_labels: Rc<RefCell<Vec<glib::WeakRef<gtk::Label>>>>,
    value: ExpressionValue,
) {
    let dialog = adw::Dialog::builder()
        .title(tr!("Expression").as_ref())
        .content_width(720)
        .content_height(460)
        .build();

    let view = editor_view(buffer, true);
    connect_editor_keys(&view, buffer, value);
    view.completion().add_provider(&words);
    track_view(&views, &view);

    let diagnostic = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .visible(false)
        .build();
    set_diagnostic_label(&diagnostic, diagnostic_message.borrow().as_deref());
    diagnostic_labels.borrow_mut().push(diagnostic.downgrade());

    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .hexpand(true)
        .vexpand(true)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&scroller);
    content.append(&diagnostic);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent.upcast_ref::<gtk::Widget>()));
    view.grab_focus();
    view.place_cursor_onscreen();
}

fn track_view(
    views: &Rc<RefCell<Vec<glib::WeakRef<sourceview5::View>>>>,
    view: &sourceview5::View,
) {
    views.borrow_mut().push(view.downgrade());
}

fn completion_words(value: ExpressionValue) -> (sourceview5::CompletionWords, sourceview5::Buffer) {
    let words = sourceview5::CompletionWords::builder()
        .title(tr!("Expression").as_ref())
        .minimum_word_size(2)
        .priority(10)
        .build();
    let seed = sourceview5::Buffer::new(None);
    seed.set_text(&rhai_editor::completion_words(value).join(" "));
    words.register(&seed);
    (words, seed)
}

fn schedule_diagnostic(
    buffer: &sourceview5::Buffer,
    tag: &gtk::TextTag,
    message: &Rc<RefCell<Option<String>>>,
    labels: &Rc<RefCell<Vec<glib::WeakRef<gtk::Label>>>>,
    pending: &Rc<RefCell<Option<SourceId>>>,
    cache: &Rc<RefCell<DiagnosticCache>>,
) {
    if let Some(source_id) = pending.borrow_mut().take() {
        source_id.remove();
    }
    let buffer = buffer.clone();
    let tag = tag.clone();
    let message = message.clone();
    let labels = labels.clone();
    let cache = cache.clone();
    let pending_for_timeout = pending.clone();
    let source_id = glib::timeout_add_local_once(
        Duration::from_millis(DIAGNOSTIC_DEBOUNCE_MILLISECONDS as u64),
        move || {
            *pending_for_timeout.borrow_mut() = None;
            update_diagnostic(&buffer, &tag, &message, &labels, &cache);
        },
    );
    *pending.borrow_mut() = Some(source_id);
}

fn update_diagnostic(
    buffer: &sourceview5::Buffer,
    tag: &gtk::TextTag,
    message: &Rc<RefCell<Option<String>>>,
    labels: &Rc<RefCell<Vec<glib::WeakRef<gtk::Label>>>>,
    cache: &Rc<RefCell<DiagnosticCache>>,
) {
    let (start, end) = buffer.bounds();
    buffer.remove_tag(tag, &start, &end);
    let source = buffer.text(&start, &end, true).to_string();
    let diagnostic = cache.borrow_mut().diagnostic(&source).cloned();
    if let Some(diagnostic) = &diagnostic {
        tag.set_priority(buffer.tag_table().size() - 1);
        apply_diagnostic_tag(buffer, tag, diagnostic.line, diagnostic.column);
    }
    *message.borrow_mut() = diagnostic.map(|diagnostic| diagnostic.message);
    set_diagnostic_labels(labels, message.borrow().as_deref());
}

fn trigger_completion(
    buffer: &sourceview5::Buffer,
    views: &Rc<RefCell<Vec<glib::WeakRef<sourceview5::View>>>>,
    value: ExpressionValue,
) {
    if current_completion(buffer, value, true).is_none() {
        return;
    }
    let mut views = views.borrow_mut();
    views.retain(|view| {
        let Some(view) = view.upgrade() else {
            return false;
        };
        view.emit_show_completion();
        true
    });
}

fn current_completion(
    buffer: &sourceview5::Buffer,
    value: ExpressionValue,
    automatic: bool,
) -> Option<rhai_editor::Completion> {
    let cursor = buffer.cursor_position();
    let offset = usize::try_from(cursor).ok()?;
    let (start, end) = buffer.bounds();
    let source = buffer.text(&start, &end, true).to_string();
    if automatic {
        rhai_editor::automatic_completion(&source, value, offset)
    } else {
        rhai_editor::completion(&source, value, offset)
    }
}

fn complete_current_prefix(buffer: &sourceview5::Buffer, value: ExpressionValue) -> bool {
    let Some(completion) = current_completion(buffer, value, false) else {
        return false;
    };
    let Some(candidate) = completion.candidates.first() else {
        return false;
    };
    let (Ok(start_offset), Ok(end_offset)) = (
        i32::try_from(completion.start),
        i32::try_from(completion.end),
    ) else {
        return false;
    };
    let mut start = buffer.iter_at_offset(start_offset);
    let mut end = buffer.iter_at_offset(end_offset);
    buffer.delete(&mut start, &mut end);
    let mut insert = buffer.iter_at_offset(start_offset);
    buffer.insert(&mut insert, candidate);
    true
}

fn apply_diagnostic_tag(
    buffer: &sourceview5::Buffer,
    tag: &gtk::TextTag,
    line: Option<usize>,
    column: Option<usize>,
) {
    let Some(line) = line else {
        let (start, end) = buffer.bounds();
        buffer.apply_tag(tag, &start, &end);
        return;
    };
    let line = line.saturating_sub(1) as i32;
    let Some(line_start) = buffer.iter_at_line(line) else {
        return;
    };
    let mut line_end = line_start;
    if !line_end.forward_to_line_end() {
        line_end = buffer.end_iter();
    }
    let mut start = column
        .and_then(|column| buffer.iter_at_line_offset(line, column.saturating_sub(1) as i32))
        .unwrap_or(line_start);
    let mut end = start;
    if end.forward_char() {
        buffer.apply_tag(tag, &start, &end);
        return;
    }
    if start.backward_char() {
        end = buffer.end_iter();
        buffer.apply_tag(tag, &start, &end);
    } else if line_start != line_end {
        buffer.apply_tag(tag, &line_start, &line_end);
    }
}

fn set_diagnostic_labels(
    labels: &Rc<RefCell<Vec<glib::WeakRef<gtk::Label>>>>,
    message: Option<&str>,
) {
    let mut labels = labels.borrow_mut();
    labels.retain(|label| {
        let Some(label) = label.upgrade() else {
            return false;
        };
        set_diagnostic_label(&label, message);
        true
    });
}

fn set_diagnostic_label(label: &gtk::Label, message: Option<&str>) {
    label.set_text(message.unwrap_or_default());
    label.set_visible(message.is_some());
    if message.is_some() {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
}

fn set_rhai_language(buffer: &sourceview5::Buffer) {
    shrimply_components_gtk::ui::configure_code_language(buffer, "rhai");
}

fn set_style_scheme(buffer: &sourceview5::Buffer) {
    let manager = sourceview5::StyleSchemeManager::default();
    let scheme_name = if adw::StyleManager::default().is_dark() {
        "Adwaita-dark"
    } else {
        "Adwaita"
    };
    if let Some(scheme) = manager.scheme(scheme_name) {
        buffer.set_style_scheme(Some(&scheme));
    }
}
