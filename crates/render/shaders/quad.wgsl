// Instanced quad + glyph pipeline.
//
// One pipeline draws everything in the scene: panels, blocks, outlines, particles and text glyphs.
// The instance buffer is the only per-frame CPU→GPU write besides the small uniform block.

struct Uniforms {
    resolution: vec2f, // physical pixels
    time: f32,         // seconds since launch
    _pad: f32,
};

struct Instance {
    pos: vec2f,    // top-left, pixels
    size: vec2f,   // pixels
    color: vec4f,  // linear-ish RGBA, 0..1
    uv_min: vec2f, // atlas rect for glyphs
    uv_max: vec2f,
    params: vec4f, // x: corner radius, y: glow, z: kind, w: pattern
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

// QuadKind in crates/render/src/scene.rs must match these numbers.
const KIND_SOLID: f32 = 0.0;
const KIND_ROUNDED: f32 = 1.0;
const KIND_BLOCK: f32 = 2.0;
const KIND_OUTLINE: f32 = 3.0;
const KIND_GRADIENT: f32 = 4.0;
const KIND_RING: f32 = 5.0;
const KIND_GLYPH: f32 = 6.0;

struct VsOut {
    @builtin(position) clip: vec4f,
    @location(0) local: vec2f, // 0..1 across the quad
    @location(1) color: vec4f,
    @location(2) uv: vec2f,
    @location(3) params: vec4f,
    @location(4) pixel: vec2f,
    @location(5) size: vec2f,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
) -> VsOut {
    var out : VsOut;
    let inst = instances[ii];

    // Triangle strip: (0,0) (0,1) (1,0) (1,1) in local space.
    let corner = vec2f(f32((vi >> 1u) & 1u), f32(vi & 1u));
    let pixel = inst.pos + corner * inst.size;

    out.local = corner;
    out.color = inst.color;
    out.params = inst.params;
    out.uv = mix(inst.uv_min, inst.uv_max, corner);
    out.pixel = pixel;
    out.size = inst.size;

    // Pixel space (y down) → clip space (y up).
    let ndc = (pixel / u.resolution) * 2.0 - vec2f(1.0, 1.0);
    out.clip = vec4f(ndc.x, -ndc.y, 0.0, 1.0);
    return out;
}

fn saturate1(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

// Signed distance to a rounded box, `p` in pixel offsets from the centre, `b` half size.
fn sd_rounded_box(p: vec2f, b: vec2f, r: f32) -> f32 {
    let q = abs(p) - b + vec2f(r, r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2f(0.0, 0.0))) - r;
}

// Antialiased coverage from a distance field (1 px feather, in distance units).
fn coverage(d: f32) -> f32 {
    let w = max(fwidth(d), 0.0001);
    return saturate1(0.5 - d / w);
}

fn lum3(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

fn shade_block(local: vec2f, size: vec2f, radius: f32, base: vec4f) -> vec4f {
    // Body: slightly darkened, with a top-to-bottom falloff.
    let vertical = 1.0 - local.y * 0.35;
    var rgb = base.rgb * (0.55 + 0.45 * vertical);

    // Bevel: highlight toward the top-left, shadow toward the bottom-right.
    let bevel = (1.0 - local.x) * (1.0 - local.y);
    rgb += base.rgb * bevel * 0.55 + vec3f(0.06);
    let shadow = local.x * local.y;
    rgb -= rgb * shadow * 0.30;

    // Emissive rim: a band just inside the edge.
    let inset = 0.06;
    let edge = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y));
    let rim = saturate1(1.0 - smoothstep(inset, inset * 3.0, edge));
    rgb += base.rgb * rim * 1.3 + vec3f(0.25) * rim;

    // Specular streak across the upper third, keeps big flat areas from looking dead.
    let streak = exp(-pow((local.y - 0.30) * 5.0, 2.0)) * saturate1(1.0 - abs(local.x - 0.35) * 1.6);
    rgb += vec3f(0.30) * streak;

    let p = local - vec2f(0.5);
    let d = sd_rounded_box(p, size * 0.5, radius);
    let a = coverage(d) * base.a;
    return vec4f(rgb, a);
}

fn pattern_factor(pattern: f32, pixel: vec2f, local: vec2f) -> f32 {
    if (pattern < 0.5) {
        return 1.0;
    }
    if (pattern < 1.5) {
        // Grid.
        let g = pixel % vec2f(24.0);
        let line = step(g.x, 1.0) + step(g.y, 1.0);
        return 1.0 + 0.35 * min(line, 1.0);
    }
    if (pattern < 2.5) {
        // Diagonal stripes.
        let s = 0.5 + 0.5 * sin((pixel.x + pixel.y) * 0.18);
        return 0.75 + 0.5 * s;
    }
    if (pattern < 3.5) {
        // Dots.
        let d = pixel % vec2f(12.0) - vec2f(6.0);
        return 1.0 + 0.6 * saturate1(1.0 - length(d) * 0.6);
    }
    // Horizontal scan bands, useful for "active" panels.
    return 0.8 + 0.4 * (0.5 + 0.5 * sin(local.y * 60.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let kind = in.params.z;
    let glow = in.params.y;
    let radius = in.params.x;
    var colour: vec4f;

    if (kind < 0.5) {
        colour = in.color;
    } else if (kind < 1.5) {
        let p = in.local - vec2f(0.5);
        let d = sd_rounded_box(p, in.size * 0.5, radius);
        colour = vec4f(in.color.rgb, coverage(d) * in.color.a);
    } else if (kind < 2.5) {
        colour = shade_block(in.local, in.size, radius, in.color);
    } else if (kind < 3.5) {
        let half = in.size * 0.5;
        let p = in.local - vec2f(0.5);
        let outer = coverage(sd_rounded_box(p, half, radius));
        let thickness = max(2.0, min(half.x, half.y) * 0.20);
        let inner = coverage(sd_rounded_box(p, half - vec2f(thickness, thickness), max(radius - thickness, 0.0)));
        colour = vec4f(in.color.rgb, (outer - inner) * in.color.a);
    } else if (kind < 4.5) {
        let top = in.color.rgb;
        let bottom = top * 0.22;
        let mixed = mix(top, bottom, in.local.y);
        colour = vec4f(mixed, in.color.a);
    } else if (kind < 5.5) {
        let p = (in.local - vec2f(0.5)) * 2.0;
        let r = length(p);
        let ring = saturate1(1.0 - abs(r - 0.78) * 7.0);
        colour = vec4f(in.color.rgb, ring * in.color.a);
    } else {
        let ink = textureSample(atlas, atlas_sampler, in.uv).a;
        colour = vec4f(in.color.rgb, ink * in.color.a);
    }

    let factor = pattern_factor(in.params.w, in.pixel, in.local);
    // Glow is an emissive push that the bloom bright-pass picks up.
    return vec4f(colour.rgb * factor * (1.0 + glow * 1.4), colour.a);
}
