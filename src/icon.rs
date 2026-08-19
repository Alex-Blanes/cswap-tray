//! Tray icon drawing: the account's initial plus two usage bars.
//!
//! Layout (16x16 actual pixels, scaled to whatever the system asks for by DPI):
//!
//! ```text
//!   ┌──────────────┐
//!   │ ██████    ▓ ░│
//!   │ ██  ██    ▓ ░│   letter    = account identity (configurable colour)
//!   │ ██████    ▓ ▓│   left bar  = 5h window
//!   │ ██        ▓ ▓│   right bar = 7d window
//!   │ ██        ▓ ▓│   filled bottom-up, green → amber → red
//!   └──────────────┘
//! ```
//!
//! The measurements are chosen so the letter box is an exact multiple of 5x7 at
//! the usual sizes (16, 20, 24, 32), which lets the bitmap font scale without
//! distortion.

/// 5x7 font. Each row uses the low 5 bits; bit 4 is the leftmost column.
const FONT: [(char, [u8; 7]); 38] = [
    ('A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    ('C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    ('D', [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110]),
    ('E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    ('F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111]),
    ('H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('I', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111]),
    ('J', [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100]),
    ('K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    ('L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    ('M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    ('N', [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001]),
    ('O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101]),
    ('R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    ('S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('V', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    ('W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001]),
    ('X', [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001]),
    ('Y', [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('!', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100]),
    ('?', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100]),
];

fn glyph(c: char) -> [u8; 7] {
    let up = c.to_ascii_uppercase();
    FONT.iter()
        .find(|(g, _)| *g == up)
        .map(|(_, bits)| *bits)
        .unwrap_or_else(|| FONT.iter().find(|(g, _)| *g == '?').unwrap().1)
}

const GREEN: [u8; 3] = [63, 185, 80];
const AMBER: [u8; 3] = [230, 160, 30];
const RED: [u8; 3] = [248, 81, 73];
const TRACK: [u8; 4] = [130, 130, 130, 120];
const GRAY: [u8; 3] = [150, 150, 150];

fn bar_color(pct: f32) -> [u8; 3] {
    if pct >= 90.0 {
        RED
    } else if pct >= 70.0 {
        AMBER
    } else {
        GREEN
    }
}

pub struct IconSpec {
    pub letter: char,
    pub color: [u8; 3],
    /// Percentages 0..100. `None` draws an empty bar (data unavailable).
    pub five_hour: Option<f32>,
    pub seven_day: Option<f32>,
    /// Degraded state: the letter is drawn in grey.
    pub stale: bool,
    /// Some account needs re-authentication: a red pip in the corner, drawn
    /// even when the account in question is not the active one.
    pub alert: bool,
}

struct Canvas {
    px: Vec<u8>,
    size: u32,
}

impl Canvas {
    fn new(size: u32) -> Self {
        Canvas { px: vec![0u8; (size * size * 4) as usize], size }
    }

    fn put(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.size || y >= self.size {
            return;
        }
        let i = ((y * self.size + x) * 4) as usize;
        self.px[i..i + 4].copy_from_slice(&rgba);
    }

    fn rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, rgba);
            }
        }
    }
}

/// Builds the icon's RGBA buffer at the requested size.
pub fn render(spec: &IconSpec, size: u32) -> Vec<u8> {
    let size = size.max(16);
    let mut c = Canvas::new(size);

    let bar_w = (size / 8).max(2);
    let gap = (size / 16).max(1);
    let x_7d = size - bar_w;
    let x_5h = x_7d - gap - bar_w;

    // --- bars -------------------------------------------------------------
    let bar_y = 1;
    let bar_h = size - 2;
    for (x, pct) in [(x_5h, spec.five_hour), (x_7d, spec.seven_day)] {
        c.rect(x, bar_y, bar_w, bar_h, TRACK);
        if let Some(p) = pct {
            let p = p.clamp(0.0, 100.0);
            let filled = ((bar_h as f32) * p / 100.0).round() as u32;
            if filled > 0 {
                let rgb = bar_color(p);
                c.rect(x, bar_y + bar_h - filled, bar_w, filled, [rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }

    // --- letter -----------------------------------------------------------
    let box_w = x_5h - gap;
    let box_h = size - 2;
    let scale = (box_w / 5).min(box_h / 7).max(1);
    let gw = 5 * scale;
    let gh = 7 * scale;
    let ox = (box_w.saturating_sub(gw)) / 2;
    let oy = 1 + (box_h.saturating_sub(gh)) / 2;
    let rgb = if spec.stale { GRAY } else { spec.color };
    let bits = glyph(spec.letter);
    for (row, line) in bits.iter().enumerate() {
        for col in 0..5u32 {
            if line & (1 << (4 - col)) != 0 {
                c.rect(
                    ox + col * scale,
                    oy + row as u32 * scale,
                    scale,
                    scale,
                    [rgb[0], rgb[1], rgb[2], 255],
                );
            }
        }
    }

    // --- alert pip --------------------------------------------------------
    // Last, so it sits on top of the letter: a dead token is worth more than
    // one corner of a glyph you can still recognise from the rest.
    if spec.alert {
        let d = (size / 8).max(2);
        c.rect(0, 0, d, d, [RED[0], RED[1], RED[2], 255]);
    }

    c.px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> IconSpec {
        IconSpec { letter: 'W', color: [59, 130, 246], five_hour: Some(50.0), seven_day: Some(100.0), stale: false, alert: false }
    }

    #[test]
    fn buffer_has_expected_length() {
        for size in [16u32, 20, 24, 32] {
            assert_eq!(render(&spec(), size).len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn glyph_scales_by_whole_pixels() {
        // With the chosen measurements the letter must fit in whole multiples.
        for (size, expected) in [(16u32, 2u32), (20, 2), (24, 3), (32, 4)] {
            let bar_w = (size / 8).max(2);
            let gap = (size / 16).max(1);
            let box_w = size - bar_w - gap - bar_w - gap;
            let scale = (box_w / 5).min((size - 2) / 7).max(1);
            assert_eq!(scale, expected, "size {size}");
        }
    }

    #[test]
    fn full_bar_paints_bottom_pixel_and_empty_leaves_track() {
        let s = IconSpec { five_hour: Some(0.0), seven_day: Some(100.0), ..spec() };
        let size = 16u32;
        let px = render(&s, size);
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        let bar_w = (size / 8).max(2);
        let x_7d = size - bar_w;
        let x_5h = x_7d - 1 - bar_w;
        assert_eq!(at(x_7d, size - 2), [RED[0], RED[1], RED[2], 255]);
        assert_eq!(at(x_5h, size - 2), TRACK);
    }

    #[test]
    fn alert_paints_the_corner_and_silence_leaves_it_alone() {
        let size = 16u32;
        let at = |px: &[u8]| [px[0], px[1], px[2], px[3]];
        let quiet = render(&IconSpec { alert: false, ..spec() }, size);
        let loud = render(&IconSpec { alert: true, ..spec() }, size);
        assert_ne!(at(&quiet), [RED[0], RED[1], RED[2], 255]);
        assert_eq!(at(&loud), [RED[0], RED[1], RED[2], 255]);
    }

    #[test]
    fn unknown_char_falls_back_to_question_mark() {
        assert_eq!(glyph('%'), glyph('?'));
    }

    /// Development aid: `cargo test preview -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_ascii() {
        for (size, letter, p5, p7) in [(16u32, 'W', 2.0, 19.0), (16, 'P', 100.0, 63.0), (32, 'P', 100.0, 63.0)] {
            let s = IconSpec {
                letter,
                color: [59, 130, 246],
                five_hour: Some(p5),
                seven_day: Some(p7),
                stale: false,
                alert: false,
            };
            let px = render(&s, size);
            println!("\n{letter} {size}x{size}  5h {p5}%  7d {p7}%");
            for y in 0..size {
                let row: String = (0..size)
                    .map(|x| {
                        let i = ((y * size + x) * 4) as usize;
                        match (px[i + 3], px[i], px[i + 1], px[i + 2]) {
                            (0, ..) => ' ',
                            (a, ..) if a < 200 => '·',
                            (_, r, g, b) if [r, g, b] == GREEN => '+',
                            (_, r, g, b) if [r, g, b] == AMBER => '*',
                            (_, r, g, b) if [r, g, b] == RED => '#',
                            _ => '@',
                        }
                    })
                    .collect();
                println!("|{row}|");
            }
        }
    }
}
