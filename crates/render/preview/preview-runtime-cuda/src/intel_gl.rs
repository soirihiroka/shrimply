use crate::gl_loader;
use glow::HasContext;
use shrimply_render_vulkan::Texture;
use std::os::fd::IntoRawFd;

const HANDLE_TYPE_OPAQUE_FD_EXT: u32 = 0x9586;
const DEDICATED_MEMORY_OBJECT_EXT: u32 = 0x9581;
const TEXTURE_TILING_EXT: u32 = 0x9580;
const OPTIMAL_TILING_EXT: i32 = 0x9584;
const LAYOUT_GENERAL_EXT: u32 = 0x958d;
const NUM_DEVICE_UUIDS_EXT: u32 = 0x9596;
const DEVICE_UUID_EXT: u32 = 0x9597;
const DRIVER_UUID_EXT: u32 = 0x9598;

pub(super) struct ImportedTexture {
    _source: Texture,
    memory: u32,
    semaphore: u32,
    pub texture: glow::NativeTexture,
    delete_memory: unsafe extern "system" fn(i32, *const u32),
    delete_semaphore: unsafe extern "system" fn(i32, *const u32),
}

impl ImportedTexture {
    pub fn new(gl: &glow::Context, source: &Texture) -> Result<Self, String> {
        for extension in ["GL_EXT_memory_object_fd", "GL_EXT_semaphore_fd"] {
            if !gl.supported_extensions().contains(extension) {
                return Err(format!(
                    "GTK OpenGL driver lacks {extension}; select the Intel OpenGL device"
                ));
            }
        }
        macro_rules! function {
            ($name:literal, $type:ty) => {{
                let address = gl_loader::proc_address($name);
                if address.is_null() {
                    return Err(format!("GTK OpenGL driver lacks {}", $name));
                }
                unsafe { std::mem::transmute::<*const std::ffi::c_void, $type>(address) }
            }};
        }
        let get_uuid = function!(
            "glGetUnsignedBytevEXT",
            unsafe extern "system" fn(u32, *mut u8)
        );
        let get_device_uuid = function!(
            "glGetUnsignedBytei_vEXT",
            unsafe extern "system" fn(u32, u32, *mut u8)
        );
        let (device_uuid, driver_uuid) = source.import_uuids();
        let mut gl_driver_uuid = [0; 16];
        unsafe { get_uuid(DRIVER_UUID_EXT, gl_driver_uuid.as_mut_ptr()) };
        let count = unsafe { gl.get_parameter_i32(NUM_DEVICE_UUIDS_EXT) };
        let same_device = (0..count.max(0) as u32).any(|index| {
            let mut uuid = [0; 16];
            unsafe { get_device_uuid(DEVICE_UUID_EXT, index, uuid.as_mut_ptr()) };
            uuid == device_uuid
        });
        if !same_device || gl_driver_uuid != driver_uuid {
            return Err(
                "GTK OpenGL and Intel Vulkan must use the same GPU and driver for preview texture sharing"
                    .into(),
            );
        }
        let error = unsafe { gl.get_error() };
        if error != glow::NO_ERROR {
            return Err(format!("Query GTK OpenGL device identity: 0x{error:x}"));
        }
        let create_memory = function!(
            "glCreateMemoryObjectsEXT",
            unsafe extern "system" fn(i32, *mut u32)
        );
        let delete_memory = function!(
            "glDeleteMemoryObjectsEXT",
            unsafe extern "system" fn(i32, *const u32)
        );
        let memory_parameter = function!(
            "glMemoryObjectParameterivEXT",
            unsafe extern "system" fn(u32, u32, *const i32)
        );
        let import_memory = function!(
            "glImportMemoryFdEXT",
            unsafe extern "system" fn(u32, u64, u32, i32)
        );
        let texture_storage = function!(
            "glTexStorageMem2DEXT",
            unsafe extern "system" fn(u32, i32, u32, i32, i32, u32, u64)
        );
        let create_semaphore = function!(
            "glGenSemaphoresEXT",
            unsafe extern "system" fn(i32, *mut u32)
        );
        let delete_semaphore = function!(
            "glDeleteSemaphoresEXT",
            unsafe extern "system" fn(i32, *const u32)
        );
        let import_semaphore = function!(
            "glImportSemaphoreFdEXT",
            unsafe extern "system" fn(u32, u32, i32)
        );
        let wait_semaphore = function!(
            "glWaitSemaphoreEXT",
            unsafe extern "system" fn(u32, u32, *const u32, u32, *const u32, *const u32)
        );
        let (memory_fd, allocation_size, semaphore_fd) = source.export()?;
        let texture = unsafe { gl.create_texture() }?;
        let mut imported = Self {
            _source: source.clone(),
            memory: 0,
            semaphore: 0,
            texture,
            delete_memory,
            delete_semaphore,
        };
        unsafe {
            create_memory(1, &mut imported.memory);
            create_semaphore(1, &mut imported.semaphore);
            memory_parameter(imported.memory, DEDICATED_MEMORY_OBJECT_EXT, &1);
            let fd = memory_fd.into_raw_fd();
            import_memory(
                imported.memory,
                allocation_size,
                HANDLE_TYPE_OPAQUE_FD_EXT,
                fd,
            );
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                libc::close(fd);
                imported.destroy(gl);
                return Err(format!(
                    "Import Intel Vulkan memory into GTK: OpenGL error 0x{error:x}"
                ));
            }
            let fd = semaphore_fd.into_raw_fd();
            import_semaphore(imported.semaphore, HANDLE_TYPE_OPAQUE_FD_EXT, fd);
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                libc::close(fd);
                imported.destroy(gl);
                return Err(format!(
                    "Import Intel Vulkan semaphore into GTK: OpenGL error 0x{error:x}"
                ));
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, TEXTURE_TILING_EXT, OPTIMAL_TILING_EXT);
            texture_storage(
                glow::TEXTURE_2D,
                source.levels() as i32,
                glow::RGBA8,
                source.width() as i32,
                source.height() as i32,
                imported.memory,
                0,
            );
            wait_semaphore(
                imported.semaphore,
                0,
                std::ptr::null(),
                1,
                &texture.0.get(),
                &LAYOUT_GENERAL_EXT,
            );
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                imported.destroy(gl);
                return Err(format!(
                    "Prepare Intel GTK presentation: OpenGL error 0x{error:x}"
                ));
            }
        }
        Ok(imported)
    }
    pub fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.texture);
            (self.delete_memory)(1, &self.memory);
            (self.delete_semaphore)(1, &self.semaphore);
        }
    }
}
