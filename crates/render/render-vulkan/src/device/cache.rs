//! Driver-produced pipeline data, shared by renderers using this Vulkan device.
use ash::vk;
#[cfg(feature = "renderer")]
use std::{collections::hash_map::DefaultHasher, fs, hash::Hasher, io::Read};
use std::{path::PathBuf, sync::OnceLock};

static PATH: OnceLock<PathBuf> = OnceLock::new();
#[cfg(feature = "renderer")]
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(feature = "renderer")]
const CHECKSUM_BYTES: usize = size_of::<u64>();
#[cfg(feature = "renderer")]
const HEADER_WORDS: usize = 4;
#[cfg(feature = "renderer")]
const HEADER_BYTES: usize = HEADER_WORDS * size_of::<u32>() + vk::UUID_SIZE;

pub(super) struct Cache {
    device: ash::Device,
    pub handle: vk::PipelineCache,
}

impl super::Device {
    /// The native host supplies a private application-cache location at startup.
    /// Other hosts can use the same cache without configuring disk persistence.
    pub fn set_pipeline_cache_path(path: PathBuf) -> Result<(), String> {
        if PATH.set(path.clone()).is_err() && PATH.get() != Some(&path) {
            return Err("Vulkan pipeline cache location cannot change after startup".into());
        }
        Ok(())
    }

    #[cfg(feature = "renderer")]
    pub(crate) fn compute_pipeline(
        &self,
        info: vk::ComputePipelineCreateInfo<'_>,
    ) -> Result<vk::Pipeline, String> {
        let mut cache = self
            .pipeline_cache
            .lock()
            .expect("Vulkan pipeline cache poisoned");
        if cache.is_none() {
            let properties = unsafe { self.instance.get_physical_device_properties(self.physical) };
            let initial = PATH
                .get()
                .and_then(|path| {
                    let file = fs::File::open(path).ok()?;
                    let mut bytes = Vec::new();
                    file.take(MAX_CACHE_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .ok()?;
                    if bytes.len() as u64 > MAX_CACHE_BYTES {
                        return None;
                    }
                    let checksum =
                        u64::from_le_bytes(bytes.get(..CHECKSUM_BYTES)?.try_into().ok()?);
                    let data = bytes.get(CHECKSUM_BYTES..)?;
                    let mut hash = DefaultHasher::new();
                    hash.write(data);
                    if checksum != hash.finish() || data.len() < HEADER_BYTES {
                        return None;
                    }
                    // Vulkan headers are tightly packed little-endian words, even
                    // on hosts with a different native byte order.
                    let header = data[..HEADER_WORDS * size_of::<u32>()]
                        .chunks_exact(size_of::<u32>())
                        .map(|bytes| {
                            u32::from_le_bytes(bytes.try_into().expect("cache header word"))
                        })
                        .collect::<Vec<_>>();
                    if header
                        != [
                            HEADER_BYTES as u32,
                            vk::PipelineCacheHeaderVersion::ONE.as_raw() as u32,
                            properties.vendor_id,
                            properties.device_id,
                        ]
                        || data[HEADER_WORDS * size_of::<u32>()..HEADER_BYTES]
                            != properties.pipeline_cache_uuid
                    {
                        return None;
                    }
                    Some(data.to_vec())
                })
                .unwrap_or_default();
            let info = vk::PipelineCacheCreateInfo::default().initial_data(&initial);
            let handle = unsafe { self.raw.create_pipeline_cache(&info, None) }
                .map_err(|error| format!("Create Vulkan pipeline cache: {error:?}"))?;
            *cache = Some(Cache {
                device: self.raw.clone(),
                handle,
            });
        }
        let cache = cache.as_ref().expect("initialized Vulkan pipeline cache");
        let pipeline = unsafe {
            self.raw
                .create_compute_pipelines(cache.handle, &[info], None)
        }
        .map_err(|(created, error)| {
            for pipeline in created {
                unsafe {
                    self.raw.destroy_pipeline(pipeline, None);
                }
            }
            format!("Compile Vulkan compute pipeline: {error:?}")
        })?[0];
        if let Some(path) = PATH.get() {
            let saved = (|| -> Result<(), String> {
                let bytes = unsafe { self.raw.get_pipeline_cache_data(cache.handle) }
                    .map_err(|error| error.to_string())?;
                if bytes.len() as u64 > MAX_CACHE_BYTES - CHECKSUM_BYTES as u64 {
                    return Ok(());
                }
                let mut hash = DefaultHasher::new();
                hash.write(&bytes);
                let data = [hash.finish().to_le_bytes().as_slice(), &bytes].concat();
                let parent = path
                    .parent()
                    .ok_or("Vulkan cache path requires a directory")?;
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                let temporary = path.with_extension("tmp");
                fs::write(&temporary, data)
                    .and_then(|_| fs::rename(temporary, path))
                    .map_err(|error| error.to_string())
            })();
            if let Err(error) = saved {
                tracing::debug!(%error, "Could not persist optional Vulkan pipeline cache");
            }
        }
        Ok(pipeline)
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline_cache(self.handle, None);
        }
    }
}
