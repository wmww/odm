use crate::grid::build_grid;
use crate::wire::mesh_edges;
use crate::{RenderError, RenderOptions, RenderScene, math};
use odm_ir::Hash;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Intermediate color targets (opaque, peel layer, accumulation): linear
/// premultiplied, float so under-compositing doesn't quantize per layer.
const ACCUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// mat4 + color + params, padded to uniform alignment.
const INSTANCE_SIZE: u64 = 96;
/// view_proj + camera_pos + viewport.
const GLOBALS_SIZE: u64 = 96;
/// Two f32x3 endpoints per wire instance.
const WIRE_STRIDE: u64 = 24;

const GRID_MINOR_COLOR: [f32; 4] = [0.16, 0.16, 0.18, 1.0];
const GRID_MAJOR_COLOR: [f32; 4] = [0.28, 0.28, 0.32, 1.0];
/// Grid line thickness in pixels (wires use `WIRE_WIDTH_PX`).
const GRID_WIDTH_PX: f32 = 1.0;

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Wire endpoint pairs (one instance per edge), built the first time this
    /// mesh is drawn as wireframe.
    wires: Option<(wgpu::Buffer, u32)>,
}

/// Renderer-owned intermediate targets, remade when the render size changes.
/// Callers only ever hand in the one final single-sample color view.
struct Targets {
    size: (u32, u32),
    /// Opaque pass: premultiplied color (cleared to the premultiplied
    /// background) + depth, both read by later passes.
    opaque_color: wgpu::TextureView,
    opaque_depth: wgpu::TextureView,
    /// Translucent accumulation, front-to-back premultiplied "under".
    accum: wgpu::TextureView,
    /// One peel layer's isolated nearest fragments.
    layer: wgpu::TextureView,
    /// Ping-pong peel depths: can't sample the depth being written.
    peel_depth: [wgpu::TextureView; 2],
    /// Peel inputs for parity p: prev peel = peel_depth[1 - p] + opaque depth.
    peel_bg: [wgpu::BindGroup; 2],
    /// `layer` for the per-layer composite.
    layer_bg: wgpu::BindGroup,
    /// `accum` + `opaque_color` for the final compose.
    compose_bg: wgpu::BindGroup,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    peel_layout: wgpu::BindGroupLayout,
    layer_layout: wgpu::BindGroupLayout,
    compose_layout: wgpu::BindGroupLayout,
    pipe_fill: wgpu::RenderPipeline,
    pipe_wire: wgpu::RenderPipeline,
    /// Peel geometry: one pipeline reused for all layers (only bind groups
    /// ping-pong) so the same triangle produces bit-identical depths.
    pipe_mesh_peel: wgpu::RenderPipeline,
    /// Layers past PEEL_LAYERS: blend under in draw order, no depth.
    pipe_mesh_tail: wgpu::RenderPipeline,
    pipe_layer_under: wgpu::RenderPipeline,
    pipe_compose: wgpu::RenderPipeline,
    instance_stride: u64,
    mesh_cache: HashMap<Hash, GpuMesh>,
    targets: Option<Targets>,
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
        let peel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("peel"),
            entries: &[depth_entry(0), depth_entry(1)],
        });
        let layer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer"),
            entries: &[texture_entry(0)],
        });
        let compose_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compose"),
            entries: &[texture_entry(1), texture_entry(2)],
        });

        let base_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("odm-base-layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&instance_layout)],
            immediate_size: 0,
        });
        let peel_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("odm-peel-layout"),
            bind_group_layouts: &[
                Some(&globals_layout),
                Some(&instance_layout),
                Some(&peel_layout),
            ],
            immediate_size: 0,
        });
        let layer_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("odm-layer-layout"),
            bind_group_layouts: &[None, None, None, Some(&layer_layout)],
            immediate_size: 0,
        });
        let compose_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("odm-compose-layout"),
            bind_group_layouts: &[None, None, None, Some(&compose_layout)],
            immediate_size: 0,
        });

        let make = |layout: &wgpu::PipelineLayout, kind: PipelineKind| {
            make_pipeline(&device, layout, &shader, pipeline_desc(kind))
        };
        let pipe_fill = make(&base_layout, PipelineKind::Fill);
        let pipe_wire = make(&base_layout, PipelineKind::Wire);
        let pipe_mesh_peel = make(&peel_pipe_layout, PipelineKind::MeshPeel);
        let pipe_mesh_tail = make(&peel_pipe_layout, PipelineKind::MeshTail);
        let pipe_layer_under = make(&layer_pipe_layout, PipelineKind::LayerUnder);
        let pipe_compose = make(&compose_pipe_layout, PipelineKind::Compose);

        let instance_stride =
            INSTANCE_SIZE.max(device.limits().min_uniform_buffer_offset_alignment as u64);

        Renderer {
            device,
            queue,
            globals_layout,
            instance_layout,
            peel_layout,
            layer_layout,
            compose_layout,
            pipe_fill,
            pipe_wire,
            pipe_mesh_peel,
            pipe_mesh_tail,
            pipe_layer_under,
            pipe_compose,
            instance_stride,
            mesh_cache: HashMap::new(),
            targets: None,
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
        let target = self.make_texture(
            size,
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let target_view = target.create_view(&Default::default());

        self.render_to_target(scene, opts, &target_view)?;
        let rgba = self.read_back(&target, opts.width, opts.height)?;
        encode_png(&rgba, opts.width, opts.height)
    }

    /// Intermediate targets for this render size, remade only on resize.
    fn ensure_targets(&mut self, width: u32, height: u32) {
        if self.targets.as_ref().is_some_and(|t| t.size == (width, height)) {
            return;
        }
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let attach_and_sample =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = |format| {
            self.make_texture(size, format, attach_and_sample).create_view(&Default::default())
        };
        let opaque_color = view(ACCUM_FORMAT);
        let opaque_depth = view(DEPTH_FORMAT);
        let accum = view(ACCUM_FORMAT);
        let layer = view(ACCUM_FORMAT);
        let peel_depth = [view(DEPTH_FORMAT), view(DEPTH_FORMAT)];

        let peel_bg = [0, 1].map(|p: usize| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("peel"),
                layout: &self.peel_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&peel_depth[1 - p]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&opaque_depth),
                    },
                ],
            })
        });
        let layer_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer"),
            layout: &self.layer_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&layer),
            }],
        });
        let compose_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compose"),
            layout: &self.compose_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&accum),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&opaque_color),
                },
            ],
        });

        self.targets = Some(Targets {
            size: (width, height),
            opaque_color,
            opaque_depth,
            accum,
            layer,
            peel_depth,
            peel_bg,
            layer_bg,
            compose_bg,
        });
    }

    /// The single scene-render path: draws into the given single-sample color
    /// view, sized `opts.width` x `opts.height`. All intermediate targets are
    /// renderer-owned. The viewer viewport uses this too.
    ///
    /// Pass list: opaque (color + depth), then depth-peeled translucency
    /// (PEEL_LAYERS exact front-to-back layers + an unsorted tail), then one
    /// fullscreen compose of accumulation over opaque into the target.
    pub fn render_to_target(
        &mut self,
        scene: &RenderScene,
        opts: &RenderOptions,
        target: &wgpu::TextureView,
    ) -> Result<(), RenderError> {
        self.ensure_targets(opts.width, opts.height);
        let cam = opts.camera.resolve(scene.bounds, opts.width as f64 / opts.height as f64);

        // Globals.
        let mut globals = [0u8; GLOBALS_SIZE as usize];
        let vp = math::to_f32_cols(&cam.view_proj);
        globals[..64].copy_from_slice(bytemuck::cast_slice(&vp));
        let eye = [cam.eye[0] as f32, cam.eye[1] as f32, cam.eye[2] as f32, 1.0f32];
        globals[64..80].copy_from_slice(bytemuck::cast_slice(&eye));
        let viewport = [opts.width as f32, opts.height as f32, 0.0, 0.0f32];
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
        // then grid minor+major. `params` is per-slot line state: x = half
        // line width in pixels (used by the wire-quad vertex path only).
        // Slot alpha is the *effective* alpha: node alpha x render opacity.
        let opacity = opts.opacity.clamp(0.0, 1.0);
        let n_inst = scene.instances.len();
        let n_slots = n_inst + 2;
        let stride = self.instance_stride as usize;
        let mut inst_data = vec![0u8; n_slots * stride];
        fn write_slot(
            data: &mut [u8],
            stride: usize,
            i: usize,
            world: &[[f32; 4]; 4],
            color: &[f32; 4],
            params: &[f32; 4],
        ) {
            let base = i * stride;
            data[base..base + 64].copy_from_slice(bytemuck::cast_slice(world));
            data[base + 64..base + 80].copy_from_slice(bytemuck::cast_slice(color));
            data[base + 80..base + 96].copy_from_slice(bytemuck::cast_slice(params));
        }
        let wire_params = [crate::WIRE_WIDTH_PX / 2.0, 0.0, 0.0, 0.0];
        let grid_params = [GRID_WIDTH_PX / 2.0, 0.0, 0.0, 0.0];
        // Partition by effective alpha: 1 -> opaque pass, <1 -> peeled.
        let mut opaque_set: Vec<usize> = Vec::with_capacity(n_inst);
        let mut translucent_set: Vec<usize> = Vec::new();
        for (i, inst) in scene.instances.iter().enumerate() {
            let world = math::to_f32_cols(&inst.world);
            let alpha = (inst.color[3] * opacity).clamp(0.0, 1.0);
            let color = [inst.color[0], inst.color[1], inst.color[2], alpha];
            write_slot(&mut inst_data, stride, i, &world, &color, &wire_params);
            if alpha >= 1.0 {
                opaque_set.push(i);
            } else if alpha > 0.0 {
                translucent_set.push(i);
            }
        }
        let identity = math::to_f32_cols(&math::IDENTITY);
        let grid_minor_slot = n_inst;
        let grid_major_slot = grid_minor_slot + 1;
        write_slot(&mut inst_data, stride, grid_minor_slot, &identity, &GRID_MINOR_COLOR, &grid_params);
        write_slot(&mut inst_data, stride, grid_major_slot, &identity, &GRID_MAJOR_COLOR, &grid_params);

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

        // Grid geometry: endpoint pairs, drawn as wire-quad instances
        // (6 floats per line = one WIRE_STRIDE instance).
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
            (minor, g.minor.len() as u32 / 6, major, g.major.len() as u32 / 6)
        });

        // Wireframe mode has no fills at all, so nothing to peel.
        let translucent_set = if opts.wireframe { Vec::new() } else { translucent_set };

        let t = self.targets.as_ref().expect("ensure_targets ran");
        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        // Opaque pass: opaque fills + (for now) all line geometry.
        {
            let bg = [
                opts.background[0] as f64 * opts.background[3] as f64,
                opts.background[1] as f64 * opts.background[3] as f64,
                opts.background[2] as f64 * opts.background[3] as f64,
                opts.background[3] as f64,
            ];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opaque"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.opaque_color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0],
                            g: bg[1],
                            b: bg[2],
                            a: bg[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.opaque_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
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
                for &i in &opaque_set {
                    let inst = &scene.instances[i];
                    let mesh = &self.mesh_cache[&inst.mesh];
                    pass.set_bind_group(1, &inst_bg, &[(i * stride) as u32]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }

            // Grid: same wire-quad path as everything line-shaped, after the
            // fill so depth testing occludes it.
            if let Some((minor_buf, minor_n, major_buf, major_n)) = &grid_bufs {
                pass.set_pipeline(&self.pipe_wire);
                pass.set_bind_group(1, &inst_bg, &[(grid_minor_slot * stride) as u32]);
                pass.set_vertex_buffer(0, minor_buf.slice(..));
                pass.draw(0..4, 0..*minor_n);
                pass.set_bind_group(1, &inst_bg, &[(grid_major_slot * stride) as u32]);
                pass.set_vertex_buffer(0, major_buf.slice(..));
                pass.draw(0..4, 0..*major_n);
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

        // Translucent peel: exact front-to-back layers, each a geometry pass
        // isolating the nearest remaining fragments (blend replace + depth
        // test) then a fullscreen under-composite; a final unsorted tail pass
        // catches anything deeper.
        let peel_layers = opts.peel_layers.max(1) as usize;
        if translucent_set.is_empty() {
            clear_color_pass(&mut encoder, &t.accum, "accum-clear");
        } else {
            // The previous-layer depth for layer 0: nothing peeled yet.
            clear_depth_pass(&mut encoder, &t.peel_depth[1], 0.0, "peel-prev-clear");

            let draw_translucent = |pass: &mut wgpu::RenderPass,
                                    mesh_cache: &HashMap<Hash, GpuMesh>| {
                for &i in &translucent_set {
                    let inst = &scene.instances[i];
                    let mesh = &mesh_cache[&inst.mesh];
                    pass.set_bind_group(1, &inst_bg, &[(i * stride) as u32]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            };

            for layer in 0..peel_layers {
                let p = layer % 2;
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("peel-layer"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &t.layer,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &t.peel_depth[p],
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.pipe_mesh_peel);
                    pass.set_bind_group(0, &globals_bg, &[]);
                    pass.set_bind_group(2, &t.peel_bg[p], &[]);
                    draw_translucent(&mut pass, &self.mesh_cache);
                }
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("peel-composite"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &t.accum,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: if layer == 0 {
                                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                                } else {
                                    wgpu::LoadOp::Load
                                },
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.pipe_layer_under);
                    pass.set_bind_group(3, &t.layer_bg, &[]);
                    pass.draw(0..3, 0..1);
                }
            }

            // Tail: whatever survives all peels blends under the accumulation
            // in draw order — a graceful degrade, never dropped geometry.
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("peel-tail"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.accum,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipe_mesh_tail);
                pass.set_bind_group(0, &globals_bg, &[]);
                pass.set_bind_group(2, &t.peel_bg[peel_layers % 2], &[]);
                draw_translucent(&mut pass, &self.mesh_cache);
            }
        }

        // Final compose into the caller's target.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("compose"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipe_compose);
            pass.set_bind_group(3, &t.compose_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn make_texture(
        &self,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
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

/// An attachment-only pass that clears `view` (used to reset the
/// accumulation and the layer-0 "previous peel" depth).
fn clear_color_pass(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, label: &str) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

fn clear_depth_pass(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: f32,
    label: &str,
) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(depth),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

enum PipelineKind {
    /// Shaded triangles, opaque pass.
    Fill,
    /// Flat-colored line quads: one instance per segment (both endpoints),
    /// widened to the slot's pixel width in the vertex shader. Serves wires
    /// and the grid alike.
    Wire,
    /// One peel layer: blend replace + depth test isolate the single nearest
    /// not-yet-peeled fragment per pixel.
    MeshPeel,
    /// Translucent geometry past the last peel layer: blend under the
    /// accumulation in draw order, no depth attachment.
    MeshTail,
    /// Fullscreen: one peel layer under the accumulation.
    LayerUnder,
    /// Fullscreen: accumulation over opaque, un-premultiplied, into the
    /// caller's target.
    Compose,
}

/// Everything that varies between pipelines.
struct PipelineDesc<'a> {
    vertex_entry: &'a str,
    fragment_entry: &'a str,
    topology: wgpu::PrimitiveTopology,
    vertex_buffers: Vec<wgpu::VertexBufferLayout<'a>>,
    /// Some(write) attaches DEPTH_FORMAT with CompareFunction::Less.
    depth: Option<bool>,
    blend: Option<wgpu::BlendState>,
    target_format: wgpu::TextureFormat,
    /// Translucent mesh passes cull back faces: solids are closed CCW
    /// shells, and a 30% box should read as one veil, not front+back
    /// composited twice.
    cull: Option<wgpu::Face>,
}

/// Front-to-back "under": dst stays in front, src fills what's left.
const BLEND_UNDER: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

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

fn depth_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn pipeline_desc<'a>(kind: PipelineKind) -> PipelineDesc<'a> {
    // Positions for meshes; line quads pull both endpoints of a segment per
    // instance instead.
    const POSITIONS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
    const WIRE_ENDS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
    let mesh_buffer = wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &POSITIONS,
    };
    let wire_buffer = wgpu::VertexBufferLayout {
        array_stride: WIRE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &WIRE_ENDS,
    };
    match kind {
        PipelineKind::Fill => PipelineDesc {
            vertex_entry: "vs_main",
            fragment_entry: "fs_mesh",
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertex_buffers: vec![mesh_buffer],
            depth: Some(true),
            blend: None,
            target_format: ACCUM_FORMAT,
            cull: None,
        },
        PipelineKind::Wire => PipelineDesc {
            vertex_entry: "vs_wire",
            fragment_entry: "fs_flat",
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            vertex_buffers: vec![wire_buffer],
            // Line quads write depth: crossing lines resolve near-first
            // instead of by draw order — matching what `pick_wire` selects.
            depth: Some(true),
            blend: None,
            target_format: ACCUM_FORMAT,
            cull: None,
        },
        PipelineKind::MeshPeel => PipelineDesc {
            vertex_entry: "vs_main",
            fragment_entry: "fs_mesh_translucent",
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertex_buffers: vec![mesh_buffer],
            depth: Some(true),
            blend: None,
            target_format: ACCUM_FORMAT,
            cull: Some(wgpu::Face::Back),
        },
        PipelineKind::MeshTail => PipelineDesc {
            vertex_entry: "vs_main",
            fragment_entry: "fs_mesh_translucent",
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertex_buffers: vec![mesh_buffer],
            depth: None,
            blend: Some(BLEND_UNDER),
            target_format: ACCUM_FORMAT,
            cull: Some(wgpu::Face::Back),
        },
        PipelineKind::LayerUnder => PipelineDesc {
            vertex_entry: "vs_fullscreen",
            fragment_entry: "fs_layer",
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertex_buffers: vec![],
            depth: None,
            blend: Some(BLEND_UNDER),
            target_format: ACCUM_FORMAT,
            cull: None,
        },
        PipelineKind::Compose => PipelineDesc {
            vertex_entry: "vs_fullscreen",
            fragment_entry: "fs_compose",
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertex_buffers: vec![],
            depth: None,
            blend: None,
            target_format: COLOR_FORMAT,
            cull: None,
        },
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    desc: PipelineDesc,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(desc.vertex_entry),
            compilation_options: Default::default(),
            buffers: &desc.vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: desc.topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: desc.cull,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: desc.depth.map(|write| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(desc.fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: desc.target_format,
                blend: desc.blend,
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
