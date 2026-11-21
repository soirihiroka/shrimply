@group(0) @binding(7) var<storage, read> instance_data: array<u32>;
// World pages point to sparse, reverse-painter-ordered instance lists. Asset
// pages and dense bricks are uploaded once, independently of integer origin.
fn instance_material_at(cell: vec3<i32>, bucket_page: u32) -> u32 {
    let bucket = bucket_page - 1u;
    let count = instance_data[bucket];
    for (var candidate = 0u; candidate < MAX_BUCKET_INSTANCES; candidate += 1u) {
        if candidate >= count { break; }
        let id = instance_data[bucket + count - candidate];
        let row = instance_data[0] + id * INSTANCE_WORDS;
        let origin = vec3<i32>(i32(instance_data[row]), i32(instance_data[row + 1u]), i32(instance_data[row + 2u]));
        let asset = instance_data[row + 3u];
        let local = cell - origin;
        let dimensions = vec3<i32>(i32(instance_data[asset]), i32(instance_data[asset + 1u]), i32(instance_data[asset + 2u]));
        if any(local < vec3<i32>(0)) || any(local >= dimensions) { continue; }
        let brick = vec3<u32>(local) / CHUNK_EDGE;
        let index = brick.x + instance_data[asset + 4u] * (brick.y + instance_data[asset + 5u] * brick.z);
        let page = instance_data[instance_data[asset + 3u] + index];
        if page == 0u { continue; }
        var material = page & 0x7fffffffu;
        if (page & 0x80000000u) == 0u {
            let voxel = vec3<u32>(local) % CHUNK_EDGE;
            let edge = CHUNK_EDGE;
            material = voxels[(page - 1u) * edge * edge * edge + voxel.x + edge * (voxel.y + edge * voxel.z)];
        }
        if material != 0u { return instance_data[instance_data[row + 4u] + material]; }
    }
    return 0u;
}
