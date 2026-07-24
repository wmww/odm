use crate::grid::build_grid;
use crate::wire::mesh_edges;
use crate::{RenderError, RenderOptions, RenderScene, math};
use odm_ir::Hash;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

pub const MSAA_SAMPLES: u32 = 4;
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// mat4 + color, padded to uniform alignment.
const INSTANCE_SIZE: u64 = 80;
/// view_proj + camera_pos + viewport.
const GLOBALS_SIZE: u64 = 96;
/// Two f32x3 endpoints per wire instance.
const WIRE_STRIDE: u64 = 24;

const GRID_MINOR_COLOR: [f32; 4] = [0.16, 0.16, 0.18, 1.0];
const GRID_MAJOR_COLOR: [f32; 4] = [0.28, 0.28, 0.32, 1.0];

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Wire endpoint pairs (one instance per edge), built the first time this
    /// mesh is drawn as wireframe.
    wires: Option<(wgpu::Buffer, u32)>,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    pipe_fill: wgpu::RenderPipeline,
    pipe_lines: wgpu::RenderPipeline,
    pipe_wire: wgpu::RenderPipeline,
    instance_stride: u64,
    mesh_cache: HashMap<Hash, GpuMesh>,
}

impl Renderer {
    /// Create with an own device on any available adapter.
    pub fn new() -> Result<Renderer, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = futures::executor::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            },
        ))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;

        let (device, queue) = futures::executor::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("odm-render"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            },
        ))
        .map_err(|e| RenderError::Device(e.to_string()))?;

        Ok(Self::with_device(device, queue))
    }

    /// Create on an existing device (e.g. eframe's) — the viewer path.
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("odm-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, false)],
        });
        let instance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instance"),
            entries: &[uniform_entry(0, true)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("odm-pipeline-layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&instance_layout)],
            immediate_size: 0,
        });

        let pipe_fill = make_pipeline(&device, &pipeline_layout, &shader, PipelineKind::Fill);
        let pipe_lines = make_pipeline(&device, &pipeline_layout, &shader, PipelineKind::Lines);
        let pipe_wire = make_pipeline(&device, &pipeline_layout, &shader, PipelineKind::Wire);

        let instance_stride =
            INSTANCE_SIZE.max(device.limits().min_uniform_buffer_offset_alignment as u64);

        Renderer {
            device,
            queue,
            globals_layout,
            instance_layout,
            pipe_fill,
            pipe_lines,
            pipe_wire,
            instance_stride,
            mesh_cache: HashMap::new(),
        }
    }

    /// Drop cached GPU buffers for meshes no longer alive.
    pub fn prune_cache(&mut self, live: &dyn Fn(&Hash) -> bool) {
        self.mesh_cache.retain(|h, _| live(h));
    }

    pub fn render_png(
        &mut self,
        scene: &RenderScene,
        opts: &RenderOptions,
    ) -> Result<Vec<u8>, RenderError> {
        if opts.width == 0 || opts.height == 0 || opts.width > 8192 || opts.height > 8192 {
            return Err(RenderError::BadOptions(format!(
                "image size {}x{} out of range (1..=8192)",
                opts.width, opts.height
            )));
        }
        let size = wgpu::Extent3d {
            width: opts.width,
            height: opts.height,
            depth_or_array_layers: 1,
        };
        let msaa = self.make_texture(size, COLOR_FORMAT, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolve = self.make_texture(
            size,
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = self.make_texture(size, DEPTH_FORMAT, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);

        let msaa_view = msaa.create_view(&Default::default());
        let resolve_view = resolve.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());

        self.render_to_views(scene, opts, &msaa_view, &resolve_view, &depth_view)?;
        let rgba = self.read_back(&resolve, opts.width, opts.height)?;
        encode_png(&rgba, opts.width, opts.height)
    }

    /// The single scene-render path: draws into the given MSAA view with
    /// resolve target. The viewer viewport uses this too.
    pub fn render_to_views(
        &mut self,
        scene: &RenderScene,
        opts: &RenderOptions,
        msaa_view: &wgpu::TextureView,
        resolve_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) -> Result<(), RenderError> {
        let cam = opts.camera.resolve(scene.bounds, opts.width as f64 / opts.height as f64);

        // Globals.
        let mut globals = [0u8; GLOBALS_SIZE as usize];
        let vp = math::to_f32_cols(&cam.view_proj);
        globals[..64].copy_from_slice(bytemuck::cast_slice(&vp));
        let eye = [cam.eye[0] as f32, cam.eye[1] as f32, cam.eye[2] as f32, 1.0f32];
        globals[64..80].copy_from_slice(bytemuck::cast_slice(&eye));
        let viewport =
            [opts.width as f32, opts.height as f32, crate::WIRE_WIDTH_PX / 2.0, 0.0f32];
        globals[80..96].copy_from_slice(bytemuck::cast_slice(&viewport));
        let globals_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: &globals,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        // Every instance must have its mesh in the scene (public API; don't
        // panic on inconsistent input).
        for inst in &scene.instances {
            if !scene.meshes.contains_key(&inst.mesh) {
                return Err(RenderError::MissingObject(inst.mesh));
            }
        }

        // Upload meshes not yet cached (edge buffers only once wireframe asks).
        let device = &self.device;
        for (hash, mesh) in &scene.meshes {
            let entry = self.mesh_cache.entry(*hash).or_insert_with(|| {
                let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh-verts"),
                    contents: bytemuck::cast_slice(&mesh.positions),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh-indices"),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                GpuMesh {
                    vertices,
                    indices,
                    index_count: mesh.indices.len() as u32,
                    wires: None,
                }
            });
            if opts.wireframe && entry.wires.is_none() {
                // Endpoints are expanded rather than indexed: each wire is one
                // instance carrying both of its ends.
                let edges = mesh_edges(mesh);
                let mut ends = Vec::with_capacity(edges.len() * 3);
                for i in &edges {
                    let v = *i as usize * 3;
                    ends.extend_from_slice(&mesh.positions[v..v + 3]);
                }
                let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh-wires"),
                    contents: bytemuck::cast_slice(&ends),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                entry.wires = Some((buf, edges.len() as u32 / 2));
            }
        }

        // Instance slots: one per instance (wires reuse the instance color),
        // then grid minor+major.
        let n_inst = scene.instances.len();
        let n_slots = n_inst + 2;
        let stride = self.instance_stride as usize;
        let mut inst_data = vec![0u8; n_slots * stride];
        fn write_slot(data: &mut [u8], stride: usize, i: usize, world: &[[f32; 4]; 4], color: &[f32; 4]) {
            let base = i * stride;
            data[base..base + 64].copy_from_slice(bytemuck::cast_slice(world));
            data[base + 64..base + 80].copy_from_slice(bytemuck::cast_slice(color));
        }
        for (i, inst) in scene.instances.iter().enumerate() {
            write_slot(&mut inst_data, stride, i, &inst.transform, &inst.color);
        }
        let identity = math::to_f32_cols(&math::IDENTITY);
        let grid_minor_slot = n_inst;
        let grid_major_slot = grid_minor_slot + 1;
        write_slot(&mut inst_data, stride, grid_minor_slot, &identity, &GRID_MINOR_COLOR);
        write_slot(&mut inst_data, stride, grid_major_slot, &identity, &GRID_MAJOR_COLOR);

        let inst_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instances"),
            contents: &inst_data,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let inst_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("instances"),
            layout: &self.instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &inst_buf,
                    offset: 0,
                    size: Some(std::num::NonZeroU64::new(INSTANCE_SIZE).unwrap()),
                }),
            }],
        });

        // Grid geometry.
        let grid = opts.grid.then(|| build_grid(scene.bounds));
        let grid_bufs = grid.as_ref().map(|g| {
            let minor = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grid-minor"),
                contents: bytemuck::cast_slice(&g.minor),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let major = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grid-major"),
                contents: bytemuck::cast_slice(&g.major),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (minor, g.minor.len() as u32 / 3, major, g.major.len() as u32 / 3)
        });

        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view,
                    depth_slice: None,
                    resolve_target: Some(resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: opts.background[0] as f64,
                            g: opts.background[1] as f64,
                            b: opts.background[2] as f64,
                            a: opts.background[3] as f64,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_bind_group(0, &globals_bg, &[]);

            // Solid fill (shaded). Wireframe mode replaces it entirely.
            if !opts.wireframe {
                pass.set_pipeline(&self.pipe_fill);
                for (i, inst) in scene.instances.iter().enumerate() {
                    let mesh = &self.mesh_cache[&inst.mesh];
                    pass.set_bind_group(1, &inst_bg, &[(i * stride) as u32]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }

            // Grid: after the fill so depth testing occludes it, before the
            // wires so they stay legible where the two cross.
            if let Some((minor_buf, minor_n, major_buf, major_n)) = &grid_bufs {
                pass.set_pipeline(&self.pipe_lines);
                pass.set_bind_group(1, &inst_bg, &[(grid_minor_slot * stride) as u32]);
                pass.set_vertex_buffer(0, minor_buf.slice(..));
                pass.draw(0..*minor_n, 0..1);
                pass.set_bind_group(1, &inst_bg, &[(grid_major_slot * stride) as u32]);
                pass.set_vertex_buffer(0, major_buf.slice(..));
                pass.draw(0..*major_n, 0..1);
            }

            // Wires, in each instance's own color, nothing hidden.
            if opts.wireframe {
                pass.set_pipeline(&self.pipe_wire);
                for (i, inst) in scene.instances.iter().enumerate() {
                    let mesh = &self.mesh_cache[&inst.mesh];
                    let Some((wire_buf, wire_count)) = &mesh.wires else { continue };
                    pass.set_bind_group(1, &inst_bg, &[(i * stride) as u32]);
                    pass.set_vertex_buffer(0, wire_buf.slice(..));
                    pass.draw(0..4, 0..*wire_count);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn make_texture(
        &self,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    fn read_back(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, RenderError> {
        let unpadded = width * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Gpu(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|_| RenderError::Gpu("map_async callback dropped".into()))?
            .map_err(|e| RenderError::Gpu(format!("map: {e}")))?;

        let data = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            rgba.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        Ok(rgba)
    }
}

enum PipelineKind {
    /// Shaded triangles.
    Fill,
    /// Flat-colored 1px line lists (the grid).
    Lines,
    /// Flat-colored wires: one instanced quad per edge, widened in screen
    /// space. Like `Lines`, depth-tested but not depth-writing, so lines
    /// never hide each other.
    Wire,
}

fn uniform_entry(binding: u32, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    kind: PipelineKind,
) -> wgpu::RenderPipeline {
    // Positions for meshes and grid lines; wire quads pull both endpoints of
    // an edge per instance instead.
    const POSITIONS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
    const WIRE_ENDS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
    let (topology, vertex_entry, fragment_entry, depth_write, vertex_buffer) = match kind {
        PipelineKind::Fill => (
            wgpu::PrimitiveTopology::TriangleList,
            "vs_main",
            "fs_mesh",
            true,
            wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &POSITIONS,
            },
        ),
        PipelineKind::Lines => (
            wgpu::PrimitiveTopology::LineList,
            "vs_main",
            "fs_flat",
            false,
            wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &POSITIONS,
            },
        ),
        PipelineKind::Wire => (
            wgpu::PrimitiveTopology::TriangleStrip,
            "vs_wire",
            "fs_flat",
            // Wires write depth: nothing else does in wireframe mode, so the
            // view stays see-through, but crossing wires resolve near-first
            // instead of by draw order — matching what `pick_wire` selects.
            true,
            wgpu::VertexBufferLayout {
                array_stride: WIRE_STRIDE,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &WIRE_ENDS,
            },
        ),
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            compilation_options: Default::default(),
            buffers: &[vertex_buffer],
        },
        primitive: wgpu::PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: MSAA_SAMPLES,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer =
            encoder.write_header().map_err(|e| RenderError::Png(e.to_string()))?;
        writer.write_image_data(rgba).map_err(|e| RenderError::Png(e.to_string()))?;
    }
    Ok(out)
}
