use egui_wgpu::{CallbackTrait, wgpu};
use std::sync::atomic::{AtomicBool, Ordering};

static INVERT_ACTIVE: AtomicBool = AtomicBool::new(false);

const INVERT_WGSL: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

struct Quad {
    v0: vec2<f32>,
    v1: vec2<f32>,
    v2: vec2<f32>,
    v3: vec2<f32>,
};

@group(0) @binding(0)
var<storage, read> quads: array<Quad>;

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32, @builtin(instance_index) i_idx: u32) -> VertexOutput {
    let q = quads[i_idx];
    var p = vec2<f32>(0.0, 0.0);
    switch (v_idx) {
        case 0u: { p = q.v0; }
        case 1u: { p = q.v1; }
        case 2u: { p = q.v2; }
        case 3u: { p = q.v0; }
        case 4u: { p = q.v2; }
        default: { p = q.v3; }
    }
    var out: VertexOutput;
    out.clip_position = vec4<f32>(p.x, p.y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuQuad {
    pub v0: [f32; 2],
    pub v1: [f32; 2],
    pub v2: [f32; 2],
    pub v3: [f32; 2],
}

pub struct InvertResources {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub storage_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub capacity: usize,
}

impl InvertResources {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("invert_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(INVERT_WGSL)),
        });

        let initial_capacity = 64;
        let storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("invert_quads_storage"),
            size: (initial_capacity * std::mem::size_of::<GpuQuad>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("invert_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("invert_bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: storage_buffer.as_entire_binding() }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("invert_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("invert_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDst,
                            dst_factor: wgpu::BlendFactor::Zero,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });

        Self { pipeline, bind_group_layout, storage_buffer, bind_group, capacity: initial_capacity }
    }

    pub fn ensure_capacity(&mut self, device: &wgpu::Device, needed: usize) {
        if needed <= self.capacity {
            return;
        }
        let new_capacity = needed.next_power_of_two();
        let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("invert_quads_storage"),
            size: (new_capacity * std::mem::size_of::<GpuQuad>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("invert_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: new_buffer.as_entire_binding() }],
        });
        self.storage_buffer = new_buffer;
        self.bind_group = new_bind_group;
        self.capacity = new_capacity;
    }
}

pub struct InvertCallback {
    pub quads: Vec<[egui::Pos2; 4]>,
}

impl CallbackTrait for InvertCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = callback_resources.get_mut::<InvertResources>() else {
            return Vec::new();
        };

        let count = self.quads.len();
        if count == 0 {
            return Vec::new();
        }

        res.ensure_capacity(device, count);

        let w = screen_descriptor.size_in_pixels[0] as f32 / screen_descriptor.pixels_per_point.max(0.001);
        let h = screen_descriptor.size_in_pixels[1] as f32 / screen_descriptor.pixels_per_point.max(0.001);
        let to_ndc = |p: egui::Pos2| -> [f32; 2] { [(p.x / w) * 2.0 - 1.0, 1.0 - (p.y / h) * 2.0] };

        let mut gpu_quads = Vec::with_capacity(count);
        for q in &self.quads {
            gpu_quads.push(GpuQuad { v0: to_ndc(q[0]), v1: to_ndc(q[1]), v2: to_ndc(q[2]), v3: to_ndc(q[3]) });
        }

        queue.write_buffer(&res.storage_buffer, 0, bytemuck::cast_slice(&gpu_quads));

        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'static>, callback_resources: &egui_wgpu::CallbackResources) {
        let count = self.quads.len();
        if count == 0 {
            return;
        }
        let Some(res) = callback_resources.get::<InvertResources>() else {
            return;
        };

        let clip = info.clip_rect_in_pixels();
        let x = clip.left_px.max(0) as u32;
        let y = clip.top_px.max(0) as u32;
        let width = (clip.width_px.max(0) as u32).min(info.screen_size_px[0].saturating_sub(x));
        let height = (clip.height_px.max(0) as u32).min(info.screen_size_px[1].saturating_sub(y));
        if width == 0 || height == 0 {
            return;
        }

        render_pass.set_viewport(0.0, 0.0, info.screen_size_px[0] as f32, info.screen_size_px[1] as f32, 0.0, 1.0);
        render_pass.set_scissor_rect(x, y, width, height);

        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, &res.bind_group, &[]);
        render_pass.draw(0..6, 0..(count as u32));
    }
}

pub fn is_active() -> bool {
    INVERT_ACTIVE.load(Ordering::Relaxed)
}

pub fn set_active(active: bool) {
    INVERT_ACTIVE.store(active, Ordering::Relaxed);
}

pub fn setup(rs: &egui_wgpu::RenderState) {
    let resources = InvertResources::new(&rs.device, rs.target_format);
    rs.renderer.write().callback_resources.insert(resources);
    set_active(true);
}

pub fn paint_invert_quads(painter: &egui::Painter, quads: &[[egui::Pos2; 4]]) -> bool {
    if quads.is_empty() || !is_active() {
        return false;
    }
    let mut min = egui::Pos2::new(f32::INFINITY, f32::INFINITY);
    let mut max = egui::Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for q in quads {
        for pt in q {
            min.x = min.x.min(pt.x);
            min.y = min.y.min(pt.y);
            max.x = max.x.max(pt.x);
            max.y = max.y.max(pt.y);
        }
    }
    let rect = egui::Rect::from_min_max(min, max);
    let cb = egui_wgpu::Callback::new_paint_callback(rect, InvertCallback { quads: quads.to_vec() });
    painter.add(cb);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_quad_size_and_alignment_match_wgsl() {
        assert_eq!(std::mem::size_of::<GpuQuad>(), 32);
        assert_eq!(std::mem::align_of::<GpuQuad>(), 4);
    }

    #[test]
    fn fallback_active_when_gpu_inactive() {
        set_active(false);
        assert!(!is_active());
        let ctx = egui::Context::default();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let quad = [egui::pos2(0.0, 0.0), egui::pos2(10.0, 0.0), egui::pos2(10.0, 10.0), egui::pos2(0.0, 10.0)];
        let painted = paint_invert_quads(&painter, &[quad]);
        assert!(!painted, "paint_invert_quads returns false when GPU invert is inactive");
    }

    #[test]
    fn arbitrary_quad_counts_supported_without_clamping() {
        let quads: Vec<[egui::Pos2; 4]> = (0..200)
            .map(|i| {
                let y = i as f32 * 10.0;
                [egui::pos2(0.0, y), egui::pos2(100.0, y), egui::pos2(100.0, y + 8.0), egui::pos2(0.0, y + 8.0)]
            })
            .collect();
        let callback = InvertCallback { quads: quads.clone() };
        assert_eq!(callback.quads.len(), 200, "all 200 quads preserved without 64-quad truncation");
    }
}
