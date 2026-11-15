#ifdef SHRIMPLY_SOFTWARE_RAYS
[shader("compute")]
[numthreads(8,8,1)]
void obj_transport_scatter_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_scatter(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_transport_trace_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_trace(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_transport_miss_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_miss(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_transport_unwind_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_unwind(software_address.data->transport_work_data[index],pixel);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_ground_prepare_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_ground_prepare(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_ground_shadow_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_ground_shadow(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_ground_push_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    transport_ground_push(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_prepare_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_prepare(software_address.data->transport_work_data[index],pixel);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_direct_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_direct(software_address.data->transport_work_data[index],pixel);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_environment_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_environment(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_reservoir_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_reservoir(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_visibility_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_visibility(software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_finish_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_finish(software_address.data->transport_work_data[index],pixel);
}

[shader("compute")]
[numthreads(1,1,1)]
void reflect_wavefront(uniform TransportWork work,uniform SampleRayWork sample,uniform PixelAccumulator accumulation) {}

[shader("compute")]
[numthreads(1,1,1)]
void obj_sample_budget_compute()
{
    software_address.data->pass_count_data[0]=sampling_budget(scene).maximum_samples;
    software_address.data->pass_count_data[1]=OBJECT_TRANSPORT ? pbr_transport_bounce_limit(scene.pbr)+1u : 1u;
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_init_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    PixelAccumulator value;
    value.budget=sampling_budget(scene);
    value.moments.color=float4(0.0);value.moments.background=float4(0.0);
    value.moments.squared_error=float4(0.0);value.moments.count=0u;value.done=0u;
    software_address.data->accumulation_data[index]=value;
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_begin_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    PixelAccumulator accumulation=software_address.data->accumulation_data[index];
    if (accumulation.done != 0u) {software_address.data->transport_work_data[index].phase=TRANSPORT_DONE;return;}
    uint sample_index=accumulation.moments.count;
    float2 offset=accumulation.budget.maximum_samples==accumulation.budget.anti_aliasing_samples
        ? anti_aliasing_offset(sample_index,scene.toon.anti_aliasing,pixel)
        : stochastic_pixel_offset(sample_index,accumulation.budget.maximum_samples,pixel);
    sample_begin(software_address.data->sample_work_data[index],software_address.data->transport_work_data[index],
        float2(pixel)+0.5+offset,pixel,uint2(output_size.width,output_size.height),random_seed(pixel,sample_index));
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_primary_finish_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    if (software_address.data->accumulation_data[index].done != 0u) return;
    sample_primary_finish(software_address.data->sample_work_data[index],software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_ground_begin_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    if (software_address.data->accumulation_data[index].done != 0u) return;
    sample_ground_begin(software_address.data->sample_work_data[index],software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_reflection_begin_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    if (software_address.data->accumulation_data[index].done != 0u) return;
    sample_reflection_begin(software_address.data->sample_work_data[index],software_address.data->transport_work_data[index]);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_ground_finish_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    if (software_address.data->accumulation_data[index].done != 0u) return;
    sample_ground_finish(software_address.data->sample_work_data[index],software_address.data->transport_work_data[index],pixel);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_accumulate_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    PixelAccumulator* slot = &software_address.data->accumulation_data[index];
    if (slot->done != 0u) return;
    SampleRayWork sample=software_address.data->sample_work_data[index];
    if (sample.phase != SAMPLE_COMPLETE) return;
    bool done=accumulate_sample(slot->moments,
        slot->budget,sample.color,sample.background);
    slot->done=done ? 1u : 0u;
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_sample_finalize_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    SampleMoments moments=software_address.data->accumulation_data[index].moments;
    object_finish_pixel(pixel,uint2(output_size.width,output_size.height),moments.color,moments.background);
}

[shader("compute")]
[numthreads(8,8,1)]
void obj_surface_toon_finish_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel=thread_id.xy+software_address.data->wavefront_origin;
    if (thread_id.x>=software_address.data->wavefront_extent.x || thread_id.y>=software_address.data->wavefront_extent.y
        || pixel.x>=output_size.width || pixel.y>=output_size.height) return;
    uint index=thread_id.y*software_address.data->wavefront_extent.x+thread_id.x;
    surface_toon_finish(software_address.data->transport_work_data[index],pixel);
}
#endif
