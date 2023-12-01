use cgmath::{Vector2, Vector3};
use eframe::{
    egui_wgpu::{self, CallbackTrait},
    wgpu,
};
use encase::{ShaderSize, ShaderType, StorageBuffer, UniformBuffer};

#[derive(ShaderType)]
pub struct GpuCamera {
    pub position: Vector2<f32>,
    pub aspect: f32,
    pub zoom: f32,
}

#[derive(ShaderType)]
pub struct GpuCircle {
    pub position: Vector2<f32>,
    pub color: Vector3<f32>,
    pub radius: f32,
}

#[derive(ShaderType)]
struct GpuCircles<'a> {
    #[size(runtime)]
    circles: &'a [GpuCircle],
}

struct RenderState {
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    circle_render_pipeline: wgpu::RenderPipeline,
    circle_buffer_size: wgpu::BufferAddress,
    circle_buffer: wgpu::Buffer,
    circle_bind_group_layout: wgpu::BindGroupLayout,
    circle_bind_group: wgpu::BindGroup,
}

pub fn create_render_state(cc: &eframe::CreationContext) {
    let egui_wgpu::RenderState {
        ref device,
        target_format,
        ref renderer,
        ..
    } = *cc.wgpu_render_state.as_ref().unwrap();

    let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Camera Buffer"),
        size: GpuCamera::SHADER_SIZE.get(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });

    let camera_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(GpuCamera::min_size()),
                },
                count: None,
            }],
        });

    let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Camera Bind Group"),
        layout: &camera_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.as_entire_binding(),
        }],
    });

    let circle_buffer_size = GpuCircles::min_size().get();
    let circle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Circle Buffer"),
        size: circle_buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    let circle_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Circle Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: Some(GpuCircles::min_size()),
                },
                count: None,
            }],
        });

    let circle_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Circle Bind Group"),
        layout: &circle_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: circle_buffer.as_entire_binding(),
        }],
    });

    let circle_shader = device.create_shader_module(wgpu::include_wgsl!("./circle_shader.wgsl"));

    let circle_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Circle Pipeline Layout"),
        bind_group_layouts: &[&camera_bind_group_layout, &circle_bind_group_layout],
        push_constant_ranges: &[],
    });

    let circle_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Circle Render Pipeline"),
        layout: Some(&circle_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &circle_shader,
            entry_point: "vertex",
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Cw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &circle_shader,
            entry_point: "pixel",
            targets: &[Some(target_format.into())],
        }),
        multiview: None,
    });

    renderer.write().callback_resources.insert(RenderState {
        camera_buffer,
        camera_bind_group,
        circle_render_pipeline,
        circle_buffer_size,
        circle_buffer,
        circle_bind_group_layout,
        circle_bind_group,
    });
}

pub struct RenderCallback {
    pub camera: GpuCamera,
    pub circles: Vec<GpuCircle>,
}

impl CallbackTrait for RenderCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let render_state: &mut RenderState = callback_resources.get_mut().unwrap();

        {
            let mut camera_buffer = UniformBuffer::new([0u8; GpuCamera::SHADER_SIZE.get() as _]);
            camera_buffer.write(&self.camera).unwrap();
            queue.write_buffer(&render_state.camera_buffer, 0, &camera_buffer.into_inner());
        }

        {
            let mut circle_buffer =
                StorageBuffer::new(Vec::with_capacity(render_state.circle_buffer_size as _));
            circle_buffer
                .write(&GpuCircles {
                    circles: &self.circles,
                })
                .unwrap();
            let circle_buffer = circle_buffer.into_inner();

            if circle_buffer.len() as wgpu::BufferAddress > render_state.circle_buffer_size {
                render_state.circle_buffer_size = circle_buffer.len() as _;

                render_state.circle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Circle Buffer"),
                    size: render_state.circle_buffer_size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });

                render_state.circle_bind_group =
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Circle Bind Group"),
                        layout: &render_state.circle_bind_group_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: render_state.circle_buffer.as_entire_binding(),
                        }],
                    });
            }

            queue.write_buffer(&render_state.circle_buffer, 0, &circle_buffer);
        }

        Vec::new()
    }

    fn finish_prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        Vec::new()
    }

    fn paint<'a>(
        &'a self,
        _info: eframe::epaint::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        callback_resources: &'a egui_wgpu::CallbackResources,
    ) {
        let render_state: &RenderState = callback_resources.get().unwrap();

        render_pass.set_pipeline(&render_state.circle_render_pipeline);
        render_pass.set_bind_group(0, &render_state.camera_bind_group, &[]);
        render_pass.set_bind_group(1, &render_state.circle_bind_group, &[]);
        render_pass.draw(0..4, 0..self.circles.len() as _);
    }
}
