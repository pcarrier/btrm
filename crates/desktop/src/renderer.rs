use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
}

pub struct Renderer {
    rect_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    rect_bind_group: wgpu::BindGroup,
    glyph_bind_group_layout: wgpu::BindGroupLayout,
    glyph_bind_group: Option<wgpu::BindGroup>,
    atlas_texture: Option<wgpu::Texture>,
    atlas_texture_view: Option<wgpu::TextureView>,
    atlas_sampler: wgpu::Sampler,
    atlas_version: u32,
    pub format: wgpu::TextureFormat,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let rect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rect.wgsl").into()),
        });
        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyph shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/glyph.wgsl").into()),
        });

        let uniforms = Uniforms { resolution: [800.0, 600.0] };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let rect_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rect bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let glyph_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let rect_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect pl"),
            bind_group_layouts: &[&rect_bind_group_layout],
            push_constant_ranges: &[],
        });
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect pipeline"),
            layout: Some(&rect_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rect_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: 8, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &rect_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let glyph_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyph pl"),
            bind_group_layouts: &[&glyph_bind_group_layout],
            push_constant_ranges: &[],
        });
        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph pipeline"),
            layout: Some(&glyph_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &glyph_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: 8, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: 16, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &glyph_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let rect_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rect bg"),
            layout: &rect_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            rect_pipeline,
            glyph_pipeline,
            uniform_buffer,
            rect_bind_group,
            glyph_bind_group_layout,
            glyph_bind_group: None,
            atlas_texture: None,
            atlas_texture_view: None,
            atlas_sampler,
            atlas_version: 0,
            format,
        }
    }

    pub fn update_resolution(&self, queue: &wgpu::Queue, width: f32, height: f32) {
        let uniforms = Uniforms { resolution: [width, height] };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn update_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, atlas: &crate::atlas::GlyphAtlas) {
        if self.atlas_version == atlas.version && self.atlas_texture.is_some() {
            return;
        }
        let size = atlas.atlas_size;
        let needs_recreate = self.atlas_texture.is_none()
            || self.atlas_texture.as_ref().map(|t| t.size().width).unwrap_or(0) != size;

        if needs_recreate {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("atlas"),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.glyph_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glyph bg"),
                layout: &self.glyph_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.atlas_sampler) },
                ],
            }));
            self.atlas_texture_view = Some(view);
            self.atlas_texture = Some(texture);
        }

        if let Some(ref texture) = self.atlas_texture {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            );
        }
        self.atlas_version = atlas.version;
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        bg_color: [f32; 3],
        bg_verts: &[f32],
        glyph_verts: &[f32],
        cursor_verts: &[f32],
        selection_verts: &[f32],
        overlay_bg_verts: &[f32],
        overlay_glyph_verts: &[f32],
        device: &wgpu::Device,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: bg_color[0] as f64,
                        g: bg_color[1] as f64,
                        b: bg_color[2] as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        if !bg_verts.is_empty() {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bg verts"),
                contents: bytemuck::cast_slice(bg_verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_bind_group(0, &self.rect_bind_group, &[]);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..(bg_verts.len() / 6) as u32, 0..1);
        }

        if !glyph_verts.is_empty() {
            if let Some(ref bg) = self.glyph_bind_group {
                let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("glyph verts"),
                    contents: bytemuck::cast_slice(glyph_verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(0, bg, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..(glyph_verts.len() / 8) as u32, 0..1);
            }
        }

        if !cursor_verts.is_empty() {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cursor verts"),
                contents: bytemuck::cast_slice(cursor_verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_bind_group(0, &self.rect_bind_group, &[]);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..(cursor_verts.len() / 6) as u32, 0..1);
        }

        if !selection_verts.is_empty() {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("selection verts"),
                contents: bytemuck::cast_slice(selection_verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_bind_group(0, &self.rect_bind_group, &[]);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..(selection_verts.len() / 6) as u32, 0..1);
        }

        if !overlay_bg_verts.is_empty() {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("overlay bg"),
                contents: bytemuck::cast_slice(overlay_bg_verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_bind_group(0, &self.rect_bind_group, &[]);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..(overlay_bg_verts.len() / 6) as u32, 0..1);
        }

        if !overlay_glyph_verts.is_empty() {
            if let Some(ref bg) = self.glyph_bind_group {
                let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("overlay glyphs"),
                    contents: bytemuck::cast_slice(overlay_glyph_verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(0, bg, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..(overlay_glyph_verts.len() / 8) as u32, 0..1);
            }
        }
    }
}
