use shrimply_scene_3d::DenoisingMethod;

pub fn denoising_methods() -> Vec<(DenoisingMethod, &'static str)> {
    let mut methods = vec![
        (DenoisingMethod::Auto, "Auto"),
        (DenoisingMethod::Off, "Off"),
    ];
    #[cfg(target_os = "linux")]
    if super::intel_backend_selected() {
        if shrimply_render_vulkan::device::Device::get()
            .is_ok_and(|device| shrimply_denoise_oidn::vulkan_supported(device.device_uuid()))
        {
            methods.push((
                DenoisingMethod::OpenImageDenoise,
                "Open Image Denoise (GPU)",
            ));
        }
        methods.push((DenoisingMethod::Atrous, "À-trous (basic GPU fallback)"));
        return methods;
    }
    methods.push((DenoisingMethod::Optix, "OptiX"));
    if shrimply_denoise_oidn::cuda_supported(shrimply_gpu_cuda::selected_device()) {
        methods.push((
            DenoisingMethod::OpenImageDenoise,
            "Open Image Denoise (GPU)",
        ));
    }
    methods
}
