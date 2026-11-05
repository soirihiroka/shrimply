use shrimply_gpu_cuda::{CudaContext, DeviceBuffer, sys};

unsafe extern "C" {
    fn shrimply_cuda_memcpy_2d_peer(
        descriptor: *const sys::CUDA_MEMCPY2D,
        source_context: sys::CUcontext,
        destination_context: sys::CUcontext,
    ) -> sys::CUresult;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const WIDTH: usize = 37;
    const HEIGHT: usize = 19;
    const SOURCE_PITCH: usize = WIDTH + 4;
    const DESTINATION_PITCH: usize = WIDTH + 6;
    let mut args = std::env::args().skip(1);
    let source_ordinal = args.next().unwrap_or_else(|| "0".into()).parse()?;
    let destination_ordinal = args.next().unwrap_or_else(|| "0".into()).parse()?;
    let source_context = CudaContext::new(source_ordinal)?;
    let destination_context = CudaContext::new(destination_ordinal)?;
    let source_stream = source_context.default_stream();
    let destination_stream = destination_context.default_stream();
    let values = (0..SOURCE_PITCH * HEIGHT)
        .map(|index| index as u32 + 1)
        .collect::<Vec<_>>();
    for managed in [false, true] {
        source_context.bind_to_thread()?;
        let mut source = if managed {
            let mut pointer = 0;
            assert_eq!(
                unsafe {
                    sys::cuMemAllocManaged(
                        &mut pointer,
                        size_of_val(values.as_slice()),
                        sys::CUmemAttach_flags_enum_CU_MEM_ATTACH_GLOBAL,
                    )
                },
                sys::CUDA_SUCCESS
            );
            unsafe { DeviceBuffer::from_raw_parts(pointer, values.len(), source_context.clone()) }
        } else {
            DeviceBuffer::zeroed(&source_stream, values.len())?
        };
        source.copy_from_host(&source_stream, &values)?;
        destination_context.bind_to_thread()?;
        let destination =
            DeviceBuffer::<u32>::zeroed(&destination_stream, DESTINATION_PITCH * HEIGHT)?;
        // CUDA's C bridge translates this public 2D ABI into its native peer descriptor.
        let mut copy: sys::CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
        copy.srcMemoryType = if managed {
            sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED
        } else {
            sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE
        };
        copy.srcDevice = source.cu_deviceptr();
        copy.srcPitch = SOURCE_PITCH * size_of::<u32>();
        copy.dstMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE;
        copy.dstDevice = destination.cu_deviceptr();
        copy.dstPitch = DESTINATION_PITCH * size_of::<u32>();
        copy.WidthInBytes = WIDTH * size_of::<u32>();
        copy.Height = HEIGHT;
        let status = unsafe {
            if managed {
                sys::cuMemcpy2D_v2(&copy)
            } else {
                shrimply_cuda_memcpy_2d_peer(
                    &copy,
                    source_context.cu_ctx(),
                    destination_context.cu_ctx(),
                )
            }
        };
        destination_context.synchronize()?;
        assert_eq!(
            status,
            sys::CUDA_SUCCESS,
            "GPU copy failed, managed={managed}"
        );
        let pixels = destination.to_host_vec(&destination_stream)?;
        for row in 0..HEIGHT {
            assert_eq!(
                &pixels[row * DESTINATION_PITCH..row * DESTINATION_PITCH + WIDTH],
                &values[row * SOURCE_PITCH..row * SOURCE_PITCH + WIDTH]
            );
            assert!(
                pixels[row * DESTINATION_PITCH + WIDTH..(row + 1) * DESTINATION_PITCH]
                    .iter()
                    .all(|&value| value == 0)
            );
        }
        println!(
            "CUDA {source_ordinal} → {destination_ordinal} copy passed; managed={managed}, pitch and padding preserved"
        );
    }
    Ok(())
}
