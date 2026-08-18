//! Reusable offscreen render target registered as an egui texture: one final
//! color texture the renderer draws into and egui samples. The main viewport
//! and the agent activity view each own one; future render windows are one
//! `OffscreenTarget` + a camera + a scene each. The renderer owns every
//! intermediate target.

use eframe::egui;
use odm_render::wgpu;

pub struct OffscreenTarget {
    size: [u32; 2],
    /// One view for both the renderer and egui: the target is gamma-space
    /// Rgba8Unorm (the shader encodes sRGB itself), which is exactly what
    /// egui samples. Held so the texture stays alive for egui's sampling.
    target_view: wgpu::TextureView,
    tex_id: egui::TextureId,
}

impl OffscreenTarget {
    /// (Re)create `slot`'s target at `size` if it is missing or sized
    /// differently, keeping the egui texture id stable across resizes.
    /// Returns true when the target was (re)created — its contents need a
    /// fresh render.
    pub fn ensure(
        slot: &mut Option<OffscreenTarget>,
        frame: &mut eframe::Frame,
        size: [u32; 2],
    ) -> bool {
        if slot.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let rs = frame.wgpu_render_state().expect("wgpu render state");
        let device = &rs.device;
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: odm_render::COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let mut egui_renderer = rs.renderer.write();
        let tex_id = match slot.take() {
            Some(old) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &view,
                    wgpu::FilterMode::Linear,
                    old.tex_id,
                );
                old.tex_id
            }
            None => {
                egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear)
            }
        };
        *slot = Some(OffscreenTarget { size, target_view: view, tex_id });
        true
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The color view renders draw into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.target_view
    }

    /// The texture at a display size, for `egui::Image::paint_at`.
    pub fn image(&self, display_size: egui::Vec2) -> egui::load::SizedTexture {
        egui::load::SizedTexture::new(self.tex_id, display_size)
    }
}
