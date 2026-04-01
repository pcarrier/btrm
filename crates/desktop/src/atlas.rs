use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Style};

const INITIAL_ATLAS_SIZE: u32 = 2048;
const MAX_ATLAS_SIZE: u32 = 8192;
const PADDING: u32 = 1;

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub struct GlyphKey {
    pub bytes: [u8; 4],
    pub len: u8,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub wide: bool,
}

#[derive(Clone, Copy)]
pub struct GlyphSlot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct GlyphAtlas {
    font_system: FontSystem,
    swash_cache: SwashCache,
    pub pixels: Vec<u8>,
    pub atlas_size: u32,
    pub version: u32,
    cache: HashMap<GlyphKey, GlyphSlot>,
    next_x: u32,
    next_y: u32,
    row_height: u32,
    pub cell_width: f32,
    pub cell_height: f32,
    font_size: f32,
    font_family: String,
}

impl GlyphAtlas {
    pub fn new(font_family: &str, font_size: f32) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let (cell_width, cell_height) = measure_cell(&mut font_system, font_family, font_size);
        let size = INITIAL_ATLAS_SIZE;
        let pixels = vec![0u8; (size * size * 4) as usize];

        Self {
            font_system,
            swash_cache,
            pixels,
            atlas_size: size,
            version: 0,
            cache: HashMap::new(),
            next_x: 0,
            next_y: 0,
            row_height: 0,
            cell_width,
            cell_height,
            font_size,
            font_family: font_family.to_string(),
        }
    }

    pub fn set_font(&mut self, family: &str, size: f32) {
        self.font_family = family.to_string();
        self.font_size = size;
        let (cw, ch) = measure_cell(&mut self.font_system, family, size);
        self.cell_width = cw;
        self.cell_height = ch;
        self.cache.clear();
        self.next_x = 0;
        self.next_y = 0;
        self.row_height = 0;
        self.pixels.fill(0);
        self.version += 1;
    }

    pub fn ensure_glyph(&mut self, key: GlyphKey) -> GlyphSlot {
        if let Some(&slot) = self.cache.get(&key) {
            return slot;
        }

        let text = std::str::from_utf8(&key.bytes[..key.len as usize]).unwrap_or(" ");
        let col_span: u32 = if key.wide { 2 } else { 1 };
        let render_w = (self.cell_width * col_span as f32).ceil() as u32 + PADDING * 2;
        let render_h = self.cell_height.ceil() as u32 + PADDING * 2;

        self.ensure_space(render_w, render_h);

        let slot_x = self.next_x;
        let slot_y = self.next_y;

        let weight = if key.bold { Weight::BOLD } else { Weight::NORMAL };
        let style = if key.italic { Style::Italic } else { Style::Normal };

        let attrs = Attrs::new()
            .family(Family::Name(&self.font_family))
            .weight(weight)
            .style(style);

        let metrics = Metrics::new(self.font_size, self.font_size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(&mut self.font_system, Some(render_w as f32), Some(render_h as f32));
        buffer.set_text(&mut self.font_system, text, attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                if let Some(image) = self.swash_cache.get_image(&mut self.font_system, physical.cache_key) {
                    let gw = image.placement.width as u32;
                    let gh = image.placement.height as u32;
                    let gx = slot_x + PADDING + (physical.x + image.placement.left) as u32;
                    let gy = slot_y + PADDING + (physical.y - image.placement.top + self.font_size as i32) as u32;

                    for py in 0..gh {
                        for px in 0..gw {
                            let dx = gx.wrapping_add(px);
                            let dy = gy.wrapping_add(py);
                            if dx >= self.atlas_size || dy >= self.atlas_size {
                                continue;
                            }
                            let src_idx = (py * gw + px) as usize;
                            let dst_idx = ((dy * self.atlas_size + dx) * 4) as usize;
                            if dst_idx + 3 >= self.pixels.len() {
                                continue;
                            }
                            match image.content {
                                cosmic_text::SwashContent::Mask => {
                                    let a = image.data[src_idx];
                                    self.pixels[dst_idx] = 255;
                                    self.pixels[dst_idx + 1] = 255;
                                    self.pixels[dst_idx + 2] = 255;
                                    self.pixels[dst_idx + 3] = a;
                                }
                                cosmic_text::SwashContent::Color => {
                                    let si = src_idx * 4;
                                    if si + 3 < image.data.len() {
                                        self.pixels[dst_idx] = image.data[si];
                                        self.pixels[dst_idx + 1] = image.data[si + 1];
                                        self.pixels[dst_idx + 2] = image.data[si + 2];
                                        self.pixels[dst_idx + 3] = image.data[si + 3];
                                    }
                                }
                                cosmic_text::SwashContent::SubpixelMask => {
                                    let si = src_idx * 3;
                                    if si + 2 < image.data.len() {
                                        self.pixels[dst_idx] = image.data[si];
                                        self.pixels[dst_idx + 1] = image.data[si + 1];
                                        self.pixels[dst_idx + 2] = image.data[si + 2];
                                        self.pixels[dst_idx + 3] = 255;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if key.underline {
            let uy = slot_y + PADDING + self.cell_height.ceil() as u32 - 1;
            if uy < self.atlas_size {
                for px in 0..render_w.min(self.atlas_size - slot_x) {
                    let idx = ((uy * self.atlas_size + slot_x + px) * 4) as usize;
                    if idx + 3 < self.pixels.len() {
                        self.pixels[idx] = 255;
                        self.pixels[idx + 1] = 255;
                        self.pixels[idx + 2] = 255;
                        self.pixels[idx + 3] = 255;
                    }
                }
            }
        }

        let slot = GlyphSlot {
            x: slot_x,
            y: slot_y,
            width: render_w,
            height: render_h,
        };
        self.cache.insert(key, slot);

        self.next_x += render_w;
        if render_h > self.row_height {
            self.row_height = render_h;
        }
        self.version += 1;
        slot
    }

    fn ensure_space(&mut self, w: u32, h: u32) {
        if self.next_x + w > self.atlas_size {
            self.next_x = 0;
            self.next_y += self.row_height;
            self.row_height = 0;
        }
        if self.next_y + h > self.atlas_size {
            if self.atlas_size < MAX_ATLAS_SIZE {
                self.grow();
            } else {
                self.cache.clear();
                self.pixels.fill(0);
                self.next_x = 0;
                self.next_y = 0;
                self.row_height = 0;
                self.version += 1;
            }
        }
    }

    fn grow(&mut self) {
        let new_size = (self.atlas_size * 2).min(MAX_ATLAS_SIZE);
        let mut new_pixels = vec![0u8; (new_size * new_size * 4) as usize];
        for y in 0..self.atlas_size {
            let src_start = (y * self.atlas_size * 4) as usize;
            let src_end = src_start + (self.atlas_size * 4) as usize;
            let dst_start = (y * new_size * 4) as usize;
            new_pixels[dst_start..dst_start + (self.atlas_size * 4) as usize]
                .copy_from_slice(&self.pixels[src_start..src_end]);
        }
        self.pixels = new_pixels;
        self.atlas_size = new_size;
        self.version += 1;
    }
}

fn measure_cell(font_system: &mut FontSystem, family: &str, size: f32) -> (f32, f32) {
    let attrs = Attrs::new().family(Family::Name(family));
    let metrics = Metrics::new(size, size * 1.2);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(size * 4.0), Some(size * 2.0));
    buffer.set_text(font_system, "M", attrs, Shaping::Advanced);
    buffer.shape_until_scroll(font_system, false);
    let mut width = size * 0.6;
    for run in buffer.layout_runs() {
        if let Some(g) = run.glyphs.first() {
            width = g.w;
        }
    }
    (width, size * 1.2)
}
