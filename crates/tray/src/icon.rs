//! The tray icon, drawn from the app icon's shapes (`packaging/icon/vikui.svg`): its
//! check mark that reads as a "V" and its reminder dot.
//!
//! Drawn rather than decoded so the tray needs no image files or decoders.

/// How the icon is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Black shapes on transparency, for macOS to tint to the menu bar (a template
    /// image); the tile is left out, as menu bar icons are bare glyphs.
    Template,
    /// The app icon's colours: the tile, a white check and the yellow dot.
    Colour,
}

/// One shape, in the SVG's 1024-unit space.
#[derive(Clone, Copy)]
enum Shape {
    RoundedRect {
        x: f32,
        y: f32,
        size: f32,
        radius: f32,
    },
    /// A round-capped line `width` wide.
    Segment {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
    },
    Circle {
        centre: (f32, f32),
        radius: f32,
    },
}

const TILE: Shape = Shape::RoundedRect {
    x: 100.,
    y: 100.,
    size: 824.,
    radius: 185.,
};
const CHECK: [Shape; 2] = [
    Shape::Segment {
        from: (310., 540.),
        to: (460., 690.),
        width: 96.,
    },
    Shape::Segment {
        from: (460., 690.),
        to: (690., 420.),
        width: 96.,
    },
];
const DOT: (f32, f32) = (772., 252.);
const DOT_RADIUS: f32 = 72.;
/// The white ring round the dot in the app icon.
const DOT_RING: f32 = 20.;

/// Samples per pixel along each axis, for smooth edges.
const SAMPLES: u32 = 4;

/// A straight (not premultiplied) colour, each channel `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Rgba {
    const CLEAR: Self = Self {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.,
    };

    const fn opaque(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1. }
    }

    /// `self` blended `t` of the way to `other`.
    fn mix(self, other: Self, t: f32) -> Self {
        let lerp = |from: f32, to: f32| (to - from).mul_add(t, from);
        Self {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }
}

const BLACK: Rgba = Rgba::opaque(0., 0., 0.);
const WHITE: Rgba = Rgba::opaque(1., 1., 1.);
/// `#7C6CF6`, `#3B5BDB` and a yellow between the app icon's `#FFC94D` and `#F59F00`.
const TILE_TOP: Rgba = Rgba::opaque(0.486, 0.424, 0.965);
const TILE_BOTTOM: Rgba = Rgba::opaque(0.231, 0.357, 0.859);
const DOT_COLOUR: Rgba = Rgba::opaque(0.961, 0.690, 0.122);

impl Shape {
    fn contains(self, (x, y): (f32, f32)) -> bool {
        match self {
            Self::RoundedRect {
                x: left,
                y: top,
                size,
                radius,
            } => {
                let half = size / 2.;
                let (cx, cy) = (left + half, top + half);
                // Distance outside the rectangle shrunk by the corner radius.
                let dx = ((x - cx).abs() - (half - radius)).max(0.);
                let dy = ((y - cy).abs() - (half - radius)).max(0.);
                dx.hypot(dy) <= radius
            }
            Self::Segment { from, to, width } => {
                distance_to_segment((x, y), from, to) <= width / 2.
            }
            Self::Circle { centre, radius } => (x - centre.0).hypot(y - centre.1) <= radius,
        }
    }
}

fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let length = abx.mul_add(abx, aby * aby);
    let t = if length == 0. {
        0.
    } else {
        ((p.0 - a.0).mul_add(abx, (p.1 - a.1) * aby) / length).clamp(0., 1.)
    };
    let (nx, ny) = (t.mul_add(abx, a.0), t.mul_add(aby, a.1));
    (p.0 - nx).hypot(p.1 - ny)
}

/// The colour at `point` (in SVG units), or `None` where nothing is drawn.
fn colour_at(style: Style, point: (f32, f32)) -> Option<Rgba> {
    let dot = Shape::Circle {
        centre: DOT,
        radius: DOT_RADIUS,
    };
    let in_check = CHECK.iter().any(|shape| shape.contains(point));
    match style {
        Style::Template => (in_check || dot.contains(point)).then_some(BLACK),
        Style::Colour => {
            let ring = Shape::Circle {
                centre: DOT,
                radius: DOT_RADIUS + DOT_RING / 2.,
            };
            if dot.contains(point) {
                Some(DOT_COLOUR)
            } else if ring.contains(point) || in_check {
                Some(WHITE)
            } else if TILE.contains(point) {
                // The tile's diagonal gradient, top-left to bottom-right.
                let t = ((point.0 + point.1 - 200.) / (2. * 824.)).clamp(0., 1.);
                Some(TILE_TOP.mix(TILE_BOTTOM, t))
            } else {
                None
            }
        }
    }
}

/// The icon as `size` × `size` straight RGBA bytes, row by row.
#[must_use]
pub fn draw(style: Style, size: u32) -> Vec<u8> {
    // The template is a bare glyph, so it is cropped to the tile's area to fill the
    // menu bar's height; the coloured icon keeps the app icon's margin.
    let (origin, span) = match style {
        Style::Template => (100., 824.),
        Style::Colour => (0., 1024.),
    };
    #[expect(clippy::cast_precision_loss, reason = "icon sizes are tiny")]
    let (scale, samples) = (span / size as f32, (SAMPLES * SAMPLES) as f32);
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for row in 0..size {
        for col in 0..size {
            // Premultiplied while averaging, so edges don't darken.
            let mut sum = Rgba::CLEAR;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    #[expect(clippy::cast_precision_loss, reason = "icon sizes are tiny")]
                    let point = (
                        ((col * SAMPLES + sx) as f32 + 0.5).mul_add(scale / SAMPLES as f32, origin),
                        ((row * SAMPLES + sy) as f32 + 0.5).mul_add(scale / SAMPLES as f32, origin),
                    );
                    if let Some(colour) = colour_at(style, point) {
                        sum.r += colour.r * colour.a;
                        sum.g += colour.g * colour.a;
                        sum.b += colour.b * colour.a;
                        sum.a += colour.a;
                    }
                }
            }
            let straight = |channel: f32| if sum.a > 0. { channel / sum.a } else { 0. };
            pixels.extend([
                to_byte(straight(sum.r)),
                to_byte(straight(sum.g)),
                to_byte(straight(sum.b)),
                to_byte(sum.a / samples),
            ]);
        }
    }
    pixels
}

fn to_byte(value: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded and clamped to 0..=255"
    )]
    let byte = (value.clamp(0., 1.) * 255.).round() as u8;
    byte
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        pixels
            .get(at..at + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .unwrap_or_default()
    }

    #[test]
    fn template_is_black_shapes_on_transparency() {
        let size = 36;
        let pixels = draw(Style::Template, size);
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        // A corner is empty; the check's elbow and the dot (SVG (460, 670) and
        // (772, 252), cropped to the tile and scaled to 36px) are solid black.
        assert_eq!(pixel(&pixels, size, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&pixels, size, 15, 24), [0, 0, 0, 255]);
        assert_eq!(pixel(&pixels, size, 29, 6), [0, 0, 0, 255]);
        // Every pixel is black, only its alpha varies.
        assert!(pixels.chunks(4).all(|p| p.starts_with(&[0, 0, 0])));
    }

    #[test]
    fn colour_icon_has_the_tile_check_and_dot() {
        let size = 64;
        let pixels = draw(Style::Colour, size);
        // The SVG's 1024 units scaled to 64px: divided by 16.
        assert_eq!(pixel(&pixels, size, 0, 0), [0, 0, 0, 0], "outside the tile");
        let [r, _, b, a] = pixel(&pixels, size, 12, 50);
        assert_eq!(a, 255);
        assert!(b > r, "blue-violet tile: {r} {b}");
        assert_eq!(pixel(&pixels, size, 28, 41), [255, 255, 255, 255]);
        let [r, _, b, _] = pixel(&pixels, size, 48, 15);
        assert!(r > 200 && b < 100, "yellow dot: {r} {b}");
    }
}
