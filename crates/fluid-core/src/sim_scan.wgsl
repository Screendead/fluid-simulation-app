// Exclusive prefix scan over the cell counts, three passes: block scans
// with per-block sums, one scan of the sums, then the add-back that also
// seeds the scatter cursors. Blocks are 256 wide; pass two handles up to
// 256 blocks, so the grid holds at most 65,536 cells — the Rust side
// asserts it.

struct SimParams {
    box_min: vec3f,
    cell: f32,
    dims: vec3u,
    count: u32,
    h: f32,
    mass: f32,
    rho0: f32,
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read> counts: array<u32>;
@group(0) @binding(2) var<storage, read_write> starts: array<u32>;
@group(0) @binding(3) var<storage, read_write> block_sums: array<u32>;
@group(0) @binding(4) var<storage, read_write> cursors: array<u32>;

var<workgroup> shared_scan: array<u32, 256>;

fn workgroup_exclusive_scan(local: u32, value: u32) -> u32 {
    shared_scan[local] = value;
    workgroupBarrier();
    var offset = 1u;
    loop {
        if offset >= 256u {
            break;
        }
        var add = 0u;
        if local >= offset {
            add = shared_scan[local - offset];
        }
        workgroupBarrier();
        shared_scan[local] += add;
        workgroupBarrier();
        offset = offset << 1u;
    }
    var result = 0u;
    if local > 0u {
        result = shared_scan[local - 1u];
    }
    return result;
}

@compute @workgroup_size(256)
fn scan_blocks(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(local_invocation_id) local: vec3u,
    @builtin(workgroup_id) group: vec3u,
) {
    let n = arrayLength(&counts);
    var value = 0u;
    if id.x < n {
        value = counts[id.x];
    }
    let scanned = workgroup_exclusive_scan(local.x, value);
    if id.x < n {
        starts[id.x] = scanned;
    }
    if local.x == 255u {
        block_sums[group.x] = scanned + value;
    }
}

@compute @workgroup_size(256)
fn scan_sums(@builtin(local_invocation_id) local: vec3u) {
    let n = arrayLength(&block_sums);
    var value = 0u;
    if local.x < n {
        value = block_sums[local.x];
    }
    let scanned = workgroup_exclusive_scan(local.x, value);
    if local.x < n {
        block_sums[local.x] = scanned;
    }
}

@compute @workgroup_size(256)
fn add_back(@builtin(global_invocation_id) id: vec3u, @builtin(workgroup_id) group: vec3u) {
    if id.x >= arrayLength(&starts) {
        return;
    }
    let start = starts[id.x] + block_sums[group.x];
    starts[id.x] = start;
    cursors[id.x] = 0u;
}

// The solver's per-substep scan: one workgroup, each thread serial over
// its share of the cells, so the whole rebuild is one dispatch instead
// of three and the serial chain is as short as the grid allows. The
// total lands one past the last cell, so a sweep reads a cell's end as
// the next cell's start and never touches the counts.
@compute @workgroup_size(256)
fn scan_single(@builtin(local_invocation_id) local: vec3u) {
    let cells = params.dims.x * params.dims.y * params.dims.z;
    let chunk = (cells + 255u) / 256u;
    let base = local.x * chunk;
    var sum = 0u;
    for (var i = 0u; i < chunk; i++) {
        if base + i < cells {
            sum += counts[base + i];
        }
    }
    var run = workgroup_exclusive_scan(local.x, sum);
    for (var i = 0u; i < chunk; i++) {
        let c = base + i;
        if c < cells {
            starts[c] = run;
            cursors[c] = 0u;
            run += counts[c];
        }
    }
    if local.x == 255u {
        starts[cells] = run;
    }
}
