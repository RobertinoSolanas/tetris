// Post-processing chain: bright-pass → separable blur → CRT composite.
//
// Every pass draws one full-screen triangle into a texture of the same footprint as its input,
// except `fs_composite`, which writes the final image (and therefore owns the distortion).

// Layout must match `PostUniforms` in renderer.rs exactly: every vec4 on a 16-byte boundary.
struct PostUniforms {
    texel: vec2f,        // 1 / source size
    source_size: vec2f,  // source size in pixels
    params0: vec4f,      // threshold, knee, strength, time
    params1: vec4f,      // direction.x, direction.y, flash, unused
    fx: vec4f,           // scanlines, curvature, vignette, aberration (px)
};

@group(0) @binding(0) var<uniform> pu: PostUniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
// Bloom is only read by `fs_composite`; the blur/bright pipelines bind a layout without it.
@group(0) @binding(2) var bloom_src: texture_2d<f32>;
@group(0) @binding(3) var src_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> VsOut {
    // One triangle covering the [-1,1] clip box.
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(3.0, -1.0),
        vec2f(-1.0, 3.0),
    );
    var out: VsOut;
    let p = positions[vi];
    out.position = vec4f(p, 0.0, 1.0);
    // Framebuffer uv, y down to match the scene target.
    out.uv = vec2f(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

fn luminance(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

fn saturate1(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

/// Extract the bright parts of the scene with a soft knee, so glow ramps in instead of popping.
@fragment
fn fs_bright(in: VsOut) -> @location(0) vec4f {
    let c = textureSample(src, src_sampler, in.uv).rgb;
    let l = luminance(c);
    let knee = max(pu.params0.y, 0.0001);
    let soft = saturate1((l - pu.params0.x + knee) / (2.0 * knee));
    let weight = max(soft * soft * pu.params0.z, 0.0);
    return vec4f(c * weight, 1.0);
}

/// 13-tap Gaussian blur along `pu.direction` (in source pixels).
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4f {
    let offsets = array<f32, 7>(0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0);
    let weights = array<f32, 7>(0.227027, 0.1945946, 0.1216216, 0.0540540, 0.0162162, 0.0032432, 0.0004865);

    let step = pu.params1.xy * pu.texel;
    var acc = textureSample(src, src_sampler, in.uv).rgb * weights[0];
    for (var i: u32 = 1u; i < 7u; i = i + 1u) {
        let o = step * offsets[i];
        acc += textureSample(src, src_sampler, in.uv + o).rgb * weights[i];
        acc += textureSample(src, src_sampler, in.uv - o).rgb * weights[i];
    }
    return vec4f(acc, 1.0);
}

/// Barrel distortion for a CRT-ish tube. Returns uv plus a mask that fades out past the edge.
fn distort(uv: vec2f, curvature: f32) -> vec4f {
    let centred = uv * 2.0 - 1.0;
    let r2 = dot(centred, centred);
    let pulled = centred * (1.0 + curvature * r2);
    let out_uv = pulled * 0.5 + 0.5;
    let inside = min(min(out_uv.x, 1.0 - out_uv.x), min(out_uv.y, 1.0 - out_uv.y));
    let edge = smoothstep(0.0, 0.006, inside);
    return vec4f(out_uv, edge, r2);
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4f {
    let d = distort(in.uv, pu.fx.y);
    let uv = d.xy;
    let mask = d.z;

    // Chromatic aberration: push red and blue radially, by a few pixels.
    let centred = uv * 2.0 - 1.0;
    let ab = pu.fx.w * pu.texel;
    let shifted_r = uv + centred * ab;
    let shifted_b = uv - centred * ab;

    var colour = vec3f(
        textureSample(src, src_sampler, shifted_r).r,
        textureSample(src, src_sampler, uv).g,
        textureSample(src, src_sampler, shifted_b).b,
    );

    // Bloom arrives blurred at half resolution and is upsampled by the sampler. `params0.z` is the
    // strength the app asked for; the gain is deliberately modest so the halo fades out within a
    // couple of playfield cells instead of washing the whole screen.
    let bloom = clamp(textureSample(bloom_src, src_sampler, uv).rgb, vec3f(0.0), vec3f(4.0));
    colour += bloom * max(pu.params0.z, 0.0);

    // Scanlines and a faint phosphor mask, in *output* pixels so they stay crisp.
    let scan = 0.5 + 0.5 * sin(uv.y * pu.source_size.y * 3.14159265);
    colour *= 1.0 - pu.fx.x * scan;
    let phos = 0.5 + 0.5 * sin(uv.x * pu.source_size.x * 3.14159265 * 0.5);
    colour *= 1.0 - pu.fx.x * 0.25 * phos;

    // Vignette.
    let v = saturate1(1.0 - pu.fx.z * d.w);
    colour *= v;

    // Global flash (screen-clear pulses) and a touch of saturation.
    colour += vec3f(pu.params1.z);
    let l = luminance(colour);
    colour = mix(vec3f(l), colour, 1.12);

    // Tube edge: outside the glass it is black.
    colour *= mask;

    // Ordered-ish dither to kill banding in the dark gradient.
    let px = floor(in.position.xy);
    let bayer = (px.x * 3.0 + px.y * 5.0) % 8.0 / 8.0;
    colour += (bayer - 0.5) * 0.004;

    return vec4f(max(colour, vec3f(0.0)), 1.0);
}
