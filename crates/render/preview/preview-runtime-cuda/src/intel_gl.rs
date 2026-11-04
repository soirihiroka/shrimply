use crate::gl_loader;
use glow::HasContext;
use shrimply_render_vulkan::Buffer;
use std::os::fd::IntoRawFd;

const HANDLE_TYPE_OPAQUE_FD_EXT: u32 = 0x9586;
const DEDICATED_MEMORY_OBJECT_EXT: u32 = 0x9581;

pub(super) struct ImportedBuffer {
    memory: u32,
    semaphore: u32,
    buffer: glow::NativeBuffer,
    delete_memory: unsafe extern "system" fn(i32, *const u32),
    delete_semaphore: unsafe extern "system" fn(i32, *const u32),
}

impl ImportedBuffer {
    pub fn new(gl: &glow::Context, source: &Buffer) -> Result<Self, String> {
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
        let buffer_storage = function!(
            "glBufferStorageMemEXT",
            unsafe extern "system" fn(u32, isize, u32, u64)
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
        let buffer = unsafe { gl.create_buffer() }?;
        let mut imported = Self {
            memory: 0,
            semaphore: 0,
            buffer,
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
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(buffer));
            buffer_storage(
                glow::PIXEL_UNPACK_BUFFER,
                source.len() as isize,
                imported.memory,
                0,
            );
            wait_semaphore(
                imported.semaphore,
                1,
                &buffer.0.get(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            );
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
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
            gl.delete_buffer(self.buffer);
            (self.delete_memory)(1, &self.memory);
            (self.delete_semaphore)(1, &self.semaphore);
        }
    }
}
