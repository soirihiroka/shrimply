use jni::sys::{JNIEnv, jclass, jstring};

static VM: std::sync::OnceLock<jni::JavaVM> = std::sync::OnceLock::new();

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeLayout_editorPaneBounds(
    env: *mut JNIEnv,
    _class: jclass,
    width: jni::sys::jfloat,
    height: jni::sys::jfloat,
    inspector_width: jni::sys::jfloat,
    timeline_fraction: jni::sys::jfloat,
    inspector_visible: jni::sys::jboolean,
    timeline_visible: jni::sys::jboolean,
) -> jni::sys::jfloatArray {
    let bounds = shrimply_components_core::math::editor_pane_bounds(
        [width, height],
        inspector_width,
        timeline_fraction,
        inspector_visible,
        timeline_visible,
    );
    let values = bounds.as_flattened();
    unsafe {
        let result = ((**env).v1_1.NewFloatArray)(env, values.len() as i32);
        assert!(!result.is_null(), "allocate pane bounds");
        ((**env).v1_1.SetFloatArrayRegion)(env, result, 0, values.len() as i32, values.as_ptr());
        assert!(!((**env).v1_2.ExceptionCheck)(env), "write pane bounds");
        result
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_panelDividerWidth(
    _env: *mut JNIEnv,
    _class: jclass,
) -> jni::sys::jfloat {
    shrimply_components_core::layout::PANEL_DIVIDER_WIDTH
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_splitPaneExtent(
    _env: *mut JNIEnv,
    _class: jclass,
    total: jni::sys::jfloat,
    divider: jni::sys::jfloat,
    fraction: jni::sys::jfloat,
) -> jni::sys::jfloat {
    shrimply_components_core::math::split_pane_extent(total, divider, fraction)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeLayout_iconVector(
    env: *mut JNIEnv,
    _class: jclass,
    name: jstring,
) -> jstring {
    let name = unsafe { string(env, name) };
    let payload = shrimply_components_core::icons::vector_json(&name)
        .unwrap_or_else(|| panic!("unknown bundled icon: {name}"));
    let characters = payload.encode_utf16().collect::<Vec<_>>();
    unsafe {
        ((**env).v1_1.NewString)(
            env,
            characters.as_ptr(),
            characters.len().try_into().expect("icon vector fits JNI"),
        )
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_inspectorMinWidth(
    _env: *mut JNIEnv,
    _class: jclass,
) -> jni::sys::jfloat {
    shrimply_components_core::layout::INSPECTOR_MIN_WIDTH
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_inspectorMaxWidth(
    _env: *mut JNIEnv,
    _class: jclass,
    width: jni::sys::jfloat,
) -> jni::sys::jfloat {
    shrimply_components_core::layout::inspector_max_width(width)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_inspectorOverlay(
    _env: *mut JNIEnv,
    _class: jclass,
    width: jni::sys::jfloat,
) -> jni::sys::jboolean {
    shrimply_components_core::layout::inspector_overlay(width) as jni::sys::jboolean
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_transportStacked(
    _env: *mut JNIEnv,
    _class: jclass,
    width: jni::sys::jfloat,
) -> jni::sys::jboolean {
    shrimply_components_core::layout::transport_stacked(width) as jni::sys::jboolean
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeLayout_menuUsesBottomSheet(
    env: *mut JNIEnv,
    _class: jclass,
    input_source: jstring,
) -> jni::sys::jboolean {
    let source = unsafe { string(env, input_source) };
    shrimply_components_core::layout::menu_uses_bottom_sheet(&source) as jni::sys::jboolean
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_compactControls(
    _env: *mut JNIEnv,
    _class: jclass,
    width: jni::sys::jfloat,
    height: jni::sys::jfloat,
) -> jni::sys::jboolean {
    shrimply_components_core::layout::compact_controls(width, height) as jni::sys::jboolean
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_choiceSearchable(
    _env: *mut JNIEnv,
    _class: jclass,
    count: jni::sys::jint,
) -> jni::sys::jboolean {
    shrimply_components_core::selector::searchable(
        count.try_into().expect("nonnegative choice count"),
    ) as jni::sys::jboolean
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeLayout_matchingChoices(
    env: *mut JNIEnv,
    _class: jclass,
    labels: jstring,
    query: jstring,
) -> jstring {
    let labels: Vec<String> = serde_json::from_str(&unsafe { string(env, labels) })
        .expect("choice labels must be strings");
    let query = unsafe { string(env, query) };
    let indices = shrimply_components_core::selector::ranked_matching_indices(&labels, &[], &query);
    let characters = serde_json::to_string(&indices)
        .expect("serialize matching choices")
        .encode_utf16()
        .collect::<Vec<_>>();
    unsafe {
        ((**env).v1_1.NewString)(
            env,
            characters.as_ptr(),
            characters
                .len()
                .try_into()
                .expect("matching choices fit JNI"),
        )
    }
}

pub(crate) fn initialize_vm(vm: jni::JavaVM) {
    let _ = VM.set(vm);
}

pub(crate) fn format_date(seconds: i64) -> Option<String> {
    let milliseconds = seconds.checked_mul(1000)?;
    VM.get()?
        .attach_current_thread(|env| -> jni::errors::Result<String> {
            let date = env.new_object(
                jni::jni_str!("java/util/Date"),
                jni::jni_sig!("(J)V"),
                &[milliseconds.into()],
            )?;
            let formatter = env
                .call_static_method(
                    jni::jni_str!("java/text/DateFormat"),
                    jni::jni_str!("getDateTimeInstance"),
                    jni::jni_sig!("()Ljava/text/DateFormat;"),
                    &[],
                )?
                .l()?;
            let value = env
                .call_method(
                    &formatter,
                    jni::jni_str!("format"),
                    jni::jni_sig!("(Ljava/util/Date;)Ljava/lang/String;"),
                    &[(&date).into()],
                )?
                .l()?;
            env.cast_local::<jni::objects::JString>(value)?
                .try_to_string(env)
        })
        .ok()
}

unsafe fn string(env: *mut JNIEnv, value: jstring) -> String {
    assert!(!value.is_null(), "null localization string");
    unsafe {
        let length = ((**env).v1_1.GetStringLength)(env, value);
        let mut characters = vec![0; length as usize];
        ((**env).v1_2.GetStringRegion)(env, value, 0, length, characters.as_mut_ptr());
        assert!(
            !((**env).v1_2.ExceptionCheck)(env),
            "read localization string"
        );
        String::from_utf16(&characters).expect("valid localization UTF-16")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeI18n_initializeLocale(
    env: *mut JNIEnv,
    _class: jclass,
    language_tags: jstring,
) {
    let tags = unsafe { string(env, language_tags) };
    let candidates = tags.split(',').map(str::to_owned).collect::<Vec<_>>();
    shrimply_i18n::init_locales(&candidates);
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeI18n_text(
    env: *mut JNIEnv,
    _class: jclass,
    key: jstring,
    arguments: jstring,
) -> jstring {
    let key = unsafe { string(env, key) };
    let arguments = unsafe { string(env, arguments) };
    let arguments: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&arguments).expect("localization arguments must be strings");
    let arguments = arguments
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect::<Vec<_>>();
    let text = shrimply_i18n::text_args(&key, &arguments);
    let characters = text.encode_utf16().collect::<Vec<_>>();
    unsafe {
        ((**env).v1_1.NewString)(
            env,
            characters.as_ptr(),
            characters
                .len()
                .try_into()
                .expect("localization string fits JNI"),
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeColor_request(
    env: *mut JNIEnv,
    _class: jclass,
    request: jstring,
) -> jstring {
    let request: serde_json::Value =
        serde_json::from_str(&unsafe { string(env, request) }).expect("color request must be JSON");
    let response = shrimply_components_core::color::picker::request(&request)
        .unwrap_or_else(|error| serde_json::json!({"error":shrimply_i18n::text(&error)}));
    let characters = response.to_string().encode_utf16().collect::<Vec<_>>();
    unsafe {
        ((**env).v1_1.NewString)(
            env,
            characters.as_ptr(),
            characters
                .len()
                .try_into()
                .expect("color presentation fits JNI"),
        )
    }
}
