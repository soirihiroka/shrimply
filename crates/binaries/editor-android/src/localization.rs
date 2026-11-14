use jni::sys::{JNIEnv, jclass, jstring};

static VM: std::sync::OnceLock<jni::JavaVM> = std::sync::OnceLock::new();

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
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_panelAnimationDurationMillis(
    _env: *mut JNIEnv,
    _class: jclass,
) -> jni::sys::jint {
    shrimply_components_core::layout::PANEL_ANIMATION_DURATION_MS as jni::sys::jint
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeLayout_panelAnimationProgress(
    _env: *mut JNIEnv,
    _class: jclass,
    progress: jni::sys::jfloat,
) -> jni::sys::jfloat {
    shrimply_components_core::math::ease_out_cubic(f64::from(progress)) as jni::sys::jfloat
}
