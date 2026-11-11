pub(super) fn connect() -> Result<cpal::Host, String> {
    #[cfg(target_os = "linux")]
    {
        // Prefer native PipeWire, then direct PulseAudio (including pipewire-pulse).
        // Never fall through to ALSA: its PulseAudio plugin can fail while timing
        // information is unavailable: https://github.com/alsa-project/alsa-plugins/issues/65
        let mut errors = Vec::new();
        for id in [cpal::HostId::PipeWire, cpal::HostId::PulseAudio] {
            match cpal::host_from_id(id) {
                Ok(host) => return Ok(host),
                Err(error) => {
                    tracing::warn!(host = id.name(), %error, "Audio backend unavailable");
                    errors.push(format!("{}: {error}", id.name()));
                }
            }
        }
        Err(format!(
            "Could not connect to an audio server: {}",
            errors.join("; ")
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(cpal::default_host())
    }
}
