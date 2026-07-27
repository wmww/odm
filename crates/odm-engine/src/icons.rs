//! Pixel-art icons for the viewer.
//!
//! Each icon is a small PNG in `assets/icons/`, `include_bytes!`d, decoded and
//! uploaded once, then drawn as a single textured quad. Sampling is `NEAREST`
//! and [`SCALE`] is a whole number, so the art on screen is the art in the file
//! — enlarged in whole pixels if asked, never filtered. See the README next to
//! the art for the format, `scripts/icon-png.py` to edit it as text.

use crate::theme;
use eframe::egui::{
    Color32, ColorImage, Id, Pos2, Rect, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// A node with no geometry of its own.
    Empty,
    /// A node that carries a mesh.
    Mesh,
    /// A plain directory, in the Open dialog.
    Folder,
    /// A directory that is an ODM project.
    Project,
}

/// Every icon, for the tests and nothing else.
#[cfg(test)]
const ALL: [Icon; 4] = [Icon::Empty, Icon::Mesh, Icon::Folder, Icon::Project];

impl Icon {
    fn name(self) -> &'static str {
        match self {
            Icon::Empty => "empty",
            Icon::Mesh => "mesh",
            Icon::Folder => "folder",
            Icon::Project => "project",
        }
    }

    fn png(self) -> &'static [u8] {
        match self {
            Icon::Empty => include_bytes!("../assets/icons/empty.png"),
            Icon::Mesh => include_bytes!("../assets/icons/mesh.png"),
            Icon::Folder => include_bytes!("../assets/icons/folder.png"),
            Icon::Project => include_bytes!("../assets/icons/project.png"),
        }
    }
}

/// UI points per art pixel. Whole numbers only: anything else samples between
/// texels and the icon smears (same rule as the bitmap fonts).
pub const SCALE: f32 = 1.0;

/// Size an icon occupies, in UI points.
pub fn size(ui: &Ui, icon: Icon) -> Vec2 {
    let [w, h] = texture(ui, icon).size();
    vec2(w as f32, h as f32) * SCALE
}

/// Paint `icon` with its top-left at `pos`. `tint` multiplies the art, so
/// [`Color32::WHITE`] paints it as authored and anything else only darkens.
pub fn paint(ui: &Ui, icon: Icon, pos: Pos2, tint: Color32) {
    let texture = texture(ui, icon);
    let rect = Rect::from_min_size(theme::snap(ui, pos), size(ui, icon));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, tint);
}

/// The icon's texture, decoded and uploaded on first use. egui's memory owns
/// the handle from then on (dropping the last one frees the texture).
fn texture(ui: &Ui, icon: Icon) -> TextureHandle {
    let key = Id::new(("odm-icon", icon.name()));
    let ctx = ui.ctx();
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(key)) {
        return texture;
    }
    // Not inside `data_mut`: uploading takes the same context lock.
    let texture = ctx.load_texture(icon.name(), decode(icon.png()), TextureOptions::NEAREST);
    ctx.data_mut(|d| d.insert_temp(key, texture.clone()));
    texture
}

/// Decode a bundled icon. Any PNG an editor is likely to write is accepted —
/// palettes and low bit depths are expanded, then whatever channels came out
/// are widened to RGBA.
fn decode(png: &'static [u8]) -> ColorImage {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("icon: not a PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("icon: PNG too large")];
    let info = reader.next_frame(&mut buf).expect("icon: undecodable PNG");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "icon: want an 8-bit PNG");

    let channels = info.color_type.samples();
    let opaque = |px: &[u8]| -> [u8; 4] {
        match *px {
            [gray] => [gray, gray, gray, 255],
            [gray, alpha] => [gray, gray, gray, alpha],
            [r, g, b] => [r, g, b, 255],
            [r, g, b, alpha] => [r, g, b, alpha],
            _ => unreachable!("PNG has 1-4 channels"),
        }
    };
    let rgba: Vec<u8> =
        buf[..info.buffer_size()].chunks_exact(channels).flat_map(opaque).collect();
    ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bundled icon decodes, and is small enough to sit on a row of 14px
    /// text without being a lone stray pixel.
    #[test]
    fn art_is_well_formed() {
        for icon in ALL {
            let image = decode(icon.png());
            let [w, h] = [image.width(), image.height()];
            assert!((4..=16).contains(&w) && (4..=16).contains(&h), "{}: {w}x{h}", icon.name());
            assert!(
                image.pixels.iter().any(|p| p.a() != 0),
                "{}: fully transparent",
                icon.name()
            );
        }
    }
}
