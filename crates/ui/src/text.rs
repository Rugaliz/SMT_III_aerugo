//! Text layout the way the original's text component does it: glyph quads placed from the font's metrics,
//! lines broken at word boundaries, then the block anchored in the text rectangle and each line justified.
//!
//! Supported: one font, plain text (no markup), normal style, left-to-right, the overflow mode that lets
//! text run past the rectangle. Anything else is reported and laid out as plain text.

use crate::color::{Color, Color32};
use crate::font::{FontAsset, Glyph};
use crate::material::TextMaterial;

/// Alignment flags (horizontal in the low byte, vertical above).
pub mod align {
    pub const LEFT: u32 = 0x1;
    pub const CENTER: u32 = 0x2;
    pub const RIGHT: u32 = 0x4;
    pub const JUSTIFIED: u32 = 0x8;
    pub const FLUSH: u32 = 0x10;
    pub const GEOMETRY_CENTER: u32 = 0x20;
    pub const TOP: u32 = 0x100;
    pub const MIDDLE: u32 = 0x200;
    pub const BOTTOM: u32 = 0x400;
    pub const BASELINE: u32 = 0x800;
    pub const MIDLINE: u32 = 0x1000;
    pub const CAPLINE: u32 = 0x2000;
}

const LARGE_POSITIVE: f32 = 32767.0;
const LARGE_NEGATIVE: f32 = -32767.0;
const EXTENT_POSITIVE: f32 = 2147483648.0;
const EXTENT_NEGATIVE: f32 = -2147483648.0;
/// Tolerance of the rectangle width test.
const WIDTH_EPSILON: f32 = 0.0001;
/// Justified and flush lines may run this much wider before breaking.
const JUSTIFIED_WIDTH: f32 = 1.05;

/// Settings of one text component.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    pub alignment: u32,
    pub word_wrapping: bool,
    pub overflow: i32,
    /// Left, top, right, bottom.
    pub margin: [f32; 4],
    pub color: Color,
    pub override_colors: bool,
    pub kerning: bool,
    pub rich_text: bool,
    pub parse_control_characters: bool,
    pub character_spacing: f32,
    pub word_spacing: f32,
    pub line_spacing: f32,
    pub paragraph_spacing: f32,
    /// Corner colors top-left, top-right, bottom-left, bottom-right, when the gradient is on.
    pub vertex_gradient: Option<[Color; 4]>,
    pub extra_padding: bool,
    pub font_style: u32,
    pub right_to_left: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 36.0,
            alignment: align::TOP | align::LEFT,
            word_wrapping: true,
            overflow: 0,
            margin: [0.0; 4],
            color: Color::WHITE,
            override_colors: false,
            kerning: true,
            rich_text: true,
            parse_control_characters: true,
            character_spacing: 0.0,
            word_spacing: 0.0,
            line_spacing: 0.0,
            paragraph_spacing: 0.0,
            vertex_gradient: None,
            extra_padding: false,
            font_style: 0,
            right_to_left: false,
        }
    }
}

/// One glyph ready to draw, in the text's local space.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    /// Corners bottom-left, top-left, top-right, bottom-right.
    pub pos: [[f32; 2]; 4],
    /// Atlas coordinates (v up), same corner order.
    pub uv: [[f32; 2]; 4],
    /// Distance-field scale of all four vertices; negative for bold glyphs.
    pub scale: f32,
    pub colors: [Color32; 4],
    pub atlas: usize,
    /// The material set by a material tag before the glyph; `None`: the text's own material.
    pub material: Option<String>,
}

/// What the material contributes to the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialMetrics {
    /// Glyph padding in atlas pixels (see [`TextMaterial::padding`]).
    pub padding: f32,
    pub is_sdf: bool,
    /// Gradient scale and first scale ratio, when the material has a gradient scale.
    pub gradient: Option<(f32, f32)>,
}

impl MaterialMetrics {
    pub fn of(mat: &mut TextMaterial, extra_padding: bool) -> Self {
        let padding = mat.padding(extra_padding);
        let gradient =
            mat.has("_GradientScale").then(|| (mat.float("_GradientScale"), mat.float("_ScaleRatioA")));
        Self { padding, is_sdf: mat.is_sdf(), gradient }
    }
}

#[derive(Clone, Debug, Default)]
struct CharInfo {
    visible: bool,
    bl: [f32; 2],
    tl: [f32; 2],
    tr: [f32; 2],
    br: [f32; 2],
    x_advance: f32,
    ascender: f32,
    baseline: f32,
    descender: f32,
    scale: f32,
    colors: [Color32; 4],
    uv: [[f32; 2]; 4],
    atlas: usize,
    line: usize,
    material: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct LineInfo {
    /// min x, min y, max x, max y.
    extents: [f32; 4],
    max_advance: f32,
    width: f32,
    margin_left: f32,
    alignment: u32,
}

/// The running values the word-break state saves and restores.
#[derive(Clone, Copy, Debug)]
struct Run {
    count: usize,
    first_of_line: usize,
    first_visible_of_line: usize,
    last_visible_of_line: usize,
    line: usize,
    line_visible: usize,
    x_advance: f32,
    line_offset: f32,
    max_ascender: f32,
    max_descender: f32,
    max_cap_height: f32,
    max_line_ascender: f32,
    max_line_descender: f32,
    start_of_line_ascender: f32,
    extents: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct Saved {
    /// Index in the input of the character the state was saved at.
    index: isize,
    run: Run,
}

/// Lay out `text` in `rect` (`x, y, w, h` of the text's rectangle in its local space). `scale_ratio` is the
/// text's scale relative to its canvas (the product of the local y scales below the canvas).
///
/// With markup on, `<material="name">` switches the following glyphs to the material `tag_material(name)`
/// returns (a name it does not know leaves the tag as plain text), `<material="default">` back to the text's own,
/// and `</material>` to the previous one.
pub fn layout(
    text: &str,
    font: &FontAsset,
    mat: MaterialMetrics,
    tag_material: &dyn Fn(&str) -> Option<MaterialMetrics>,
    style: &TextStyle,
    rect: [f32; 4],
    scale_ratio: f32,
) -> Vec<GlyphQuad> {
    let (chars, char_material, materials) = parse_material_tags(text, style.rich_text, tag_material);
    if chars.is_empty() {
        return Vec::new();
    }
    let metrics = |m: Option<usize>| m.map_or(mat, |i| materials[i].1);
    if style.font_style != 0 || style.right_to_left || style.overflow != 0 {
        log::warn!(
            "text style {} / right-to-left {} / overflow {} not supported yet",
            style.font_style,
            style.right_to_left,
            style.overflow
        );
    }
    // Characters without a glyph in the font: the original substitutes a fallback; not ported.
    let chars: Vec<(u32, &Glyph, f32, Option<usize>)> = chars
        .iter()
        .zip(&char_material)
        .filter_map(|(&c, &m)| match font.glyph_of(c) {
            Some((g, s)) => Some((c, g, s, m)),
            None => {
                log::warn!("character U+{c:04X} is not in font {}", font.name);
                None
            }
        })
        .collect();
    if chars.is_empty() {
        return Vec::new();
    }

    let face = &font.face;
    let [rx, ry, rw, rh] = rect;
    let margin_width = rw - style.margin[0] - style.margin[2];
    let font_scale = face.scale * (style.font_size / face.point_size);
    let base_scale = font_scale;
    let line_gap = face.line_height - (face.ascent_line - face.descent_line);
    let font_color = Color32::from(style.color);
    // Margins and indents set by markup: always zero without it.
    let (tag_margin_left, tag_margin_right, tag_indent, tag_line_indent) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let width_of_area = margin_width + WIDTH_EPSILON - tag_margin_left - tag_margin_right;
    // Per-character spacing set by markup.
    let c_spacing = 0.0f32;

    let mut info: Vec<CharInfo> = vec![CharInfo::default(); chars.len()];
    let mut lines: Vec<LineInfo> = vec![LineInfo::default()];
    let mut run = Run {
        count: 0,
        first_of_line: 0,
        first_visible_of_line: 0,
        last_visible_of_line: 0,
        line: 0,
        line_visible: 0,
        x_advance: 0.0,
        line_offset: 0.0,
        max_ascender: 0.0,
        max_descender: 0.0,
        max_cap_height: 0.0,
        max_line_ascender: LARGE_NEGATIVE,
        max_line_descender: LARGE_POSITIVE,
        start_of_line_ascender: 0.0,
        extents: [EXTENT_POSITIVE, EXTENT_POSITIVE, EXTENT_NEGATIVE, EXTENT_NEGATIVE],
    };
    let mut wrap = Saved { index: -1, run };
    let mut max_visible_descender = 0.0f32;
    let mut start_of_new_line = true;
    let mut first_word = true;
    let mut ignore_non_breaking = false;
    let mut char_wrapping = false;
    let mut last_breaking = false;
    let mut last_restored: isize = 0;

    let last = chars.len() - 1;
    let mut i = 0usize;
    while i < chars.len() {
        let (c, glyph, char_scale, char_mat) = chars[i];
        let mat = metrics(char_mat);
        let n = run.count;
        info[n] = CharInfo { material: char_mat, ..CharInfo::default() };
        let element_scale = glyph.scale * (font_scale * char_scale);
        info[n].scale = element_scale;
        let mut padding = mat.padding;
        let character_spacing = style.character_spacing;
        // Normal style: style padding from the font's normal weight.
        let style_padding = match mat.gradient {
            Some((gs, ratio_a)) => {
                let sp = font.normal_style / 4.0 * gs * ratio_a;
                if sp + padding > gs {
                    padding = gs - sp;
                }
                sp
            }
            None => 0.0,
        };
        let bold_factor = 1.0;
        let width_adj = 1.0;
        // Kerning: the fonts carry no pair adjustments.
        let (adj_x, adj_y, adj_advance) = (0.0f32, 0.0f32, 0.0f32);

        // Quad corners.
        let baseline_local = face.baseline * font_scale * face.scale;
        let tl = [
            run.x_advance + (glyph.bearing_x - padding - style_padding + adj_x) * element_scale * width_adj,
            (glyph.bearing_y + padding + adj_y) * element_scale + baseline_local - run.line_offset,
        ];
        let bl = [tl[0], tl[1] - (glyph.height + padding * 2.0) * element_scale];
        let tr =
            [bl[0] + (glyph.width + padding * 2.0 + style_padding * 2.0) * element_scale * width_adj, tl[1]];
        let br = [tr[0], bl[1]];
        let ch = &mut info[n];
        (ch.bl, ch.tl, ch.tr, ch.br) = (bl, tl, tr, br);
        ch.baseline = baseline_local - run.line_offset;

        // Ascender and descender.
        let element_ascender = face.ascent_line * element_scale;
        ch.ascender = element_ascender - run.line_offset;
        run.max_line_ascender = run.max_line_ascender.max(element_ascender);
        let element_descender = face.descent_line * element_scale;
        ch.descender = element_descender - run.line_offset;
        run.max_line_descender = run.max_line_descender.min(element_descender);
        if run.line == 0 {
            run.max_ascender = run.max_ascender.max(element_ascender);
            run.max_cap_height = run.max_cap_height.max(face.cap_line * element_scale);
        }

        // Visible characters: the width test, line breaking and the glyph's vertices.
        let visible = c == 9 || c == 0xA0 || c == 0x2007 || (!is_whitespace(c) && c != 0x200B);
        if visible {
            info[n].visible = true;
            lines[run.line].margin_left = tag_margin_left;
            let limit_scale = if lines_justified(style.alignment) { JUSTIFIED_WIDTH } else { 1.0 };
            let text_width = run.x_advance.abs() + glyph.advance * width_adj * element_scale;
            // Too wide: break at the last word boundary when wrapping (otherwise the overflow mode decides;
            // mode 0 lets the text run on).
            if text_width > width_of_area * limit_scale
                && style.word_wrapping
                && run.count != run.first_of_line
            {
                if last_restored == wrap.index || first_word {
                    // A word longer than the line: retry ignoring non-breaking spaces, then by character.
                    if char_wrapping {
                        last_breaking = true;
                    } else if ignore_non_breaking {
                        char_wrapping = true;
                    } else {
                        ignore_non_breaking = true;
                    }
                }
                i = wrap.index as usize;
                run = wrap.run;
                last_restored = i as isize;
                if run.line > 0 && !approximately(run.max_line_ascender, run.start_of_line_ascender) {
                    let offset = run.max_line_ascender - run.start_of_line_ascender;
                    shift_line(&mut info, run.first_of_line, run.count, offset);
                    run.line_offset += offset;
                    wrap.run.line_offset = run.line_offset;
                    wrap.run.start_of_line_ascender = run.max_line_ascender;
                }
                close_line(&mut lines[run.line], &info, &mut run, &mut max_visible_descender);
                lines[run.line].max_advance = info[run.last_visible_of_line].x_advance
                    - (character_spacing + font.normal_spacing) * element_scale
                    - c_spacing;
                lines[run.line].width = width_of_area;
                run.first_of_line = run.count;
                run.line_visible = 0;
                run.line += 1;
                start_of_new_line = true;
                first_word = true;
                if lines.len() <= run.line {
                    lines.resize(run.line + 1, LineInfo::default());
                }
                // The character the break was saved at starts the new line's ascender.
                let ascender = info[run.count].ascender - info[run.count].baseline;
                run.line_offset +=
                    0.0 - run.max_line_descender + ascender + (line_gap + style.line_spacing) * base_scale;
                run.start_of_line_ascender = ascender;
                run.max_line_ascender = LARGE_NEGATIVE;
                run.max_line_descender = LARGE_POSITIVE;
                run.x_advance = tag_indent;
                i += 1;
                continue;
            }
            let vertex_color = font_color;
            save_glyph(
                &mut info[n],
                glyph,
                font,
                padding,
                style_padding,
                mat.is_sdf,
                vertex_color,
                font_color,
                style,
            );
        }
        if info[n].visible && c != 0xAD {
            if start_of_new_line {
                run.first_visible_of_line = n;
                start_of_new_line = false;
            }
            run.line_visible += 1;
            run.last_visible_of_line = n;
        }

        // A taller character than the line's first ones moves the whole line down.
        if run.line > 0 && !approximately(run.max_line_ascender, run.start_of_line_ascender) {
            let offset = run.max_line_ascender - run.start_of_line_ascender;
            shift_line(&mut info, run.first_of_line, run.count, offset);
            run.line_offset += offset;
            run.start_of_line_ascender += offset;
            wrap.run.line_offset = run.line_offset;
            wrap.run.start_of_line_ascender = run.start_of_line_ascender;
        }
        info[n].line = run.line;
        if (c != 10 && c != 13 && c != 0x2026) || n + 1 - run.first_of_line.min(n + 1) == 1 {
            lines[run.line].alignment = style.alignment;
        }

        // Advance.
        if c == 9 {
            let tab = font.tab_size * face.tab_width * element_scale;
            let next = (run.x_advance / tab).ceil() * tab;
            run.x_advance = if next > run.x_advance { next } else { run.x_advance + tab };
        } else {
            run.x_advance +=
                ((glyph.advance * bold_factor + character_spacing + font.normal_spacing + adj_advance)
                    * element_scale
                    + c_spacing)
                    * width_adj;
            if is_whitespace(c) || c == 0x200B {
                run.x_advance += style.word_spacing * element_scale;
            }
        }
        info[n].x_advance = run.x_advance;
        if c == 13 {
            run.x_advance = tag_indent;
        }

        // End of a line: a line feed or the last character.
        if c == 10 || i == last {
            if run.line > 0 && !approximately(run.max_line_ascender, run.start_of_line_ascender) {
                let offset = run.max_line_ascender - run.start_of_line_ascender;
                shift_line(&mut info, run.first_of_line, run.count, offset);
                run.line_offset += offset;
            }
            close_line(&mut lines[run.line], &info, &mut run, &mut max_visible_descender);
            let end = if info[run.last_visible_of_line].visible { run.last_visible_of_line } else { n };
            lines[run.line].max_advance =
                info[end].x_advance - (character_spacing + font.normal_spacing) * element_scale - c_spacing;
            lines[run.line].width = width_of_area;
            if n + 1 - run.first_of_line.min(n + 1) == 1 {
                lines[run.line].alignment = style.alignment;
            }
            run.first_of_line = n + 1;
            run.line_visible = 0;
            if c == 10 {
                wrap = Saved { index: i as isize, run };
                run.line += 1;
                start_of_new_line = true;
                ignore_non_breaking = false;
                first_word = true;
                if lines.len() <= run.line {
                    lines.resize(run.line + 1, LineInfo::default());
                }
                run.line_offset += 0.0 - run.max_line_descender
                    + element_ascender
                    + (line_gap + style.line_spacing + style.paragraph_spacing) * base_scale;
                run.max_line_ascender = LARGE_NEGATIVE;
                run.max_line_descender = LARGE_POSITIVE;
                run.start_of_line_ascender = element_ascender;
                run.x_advance = tag_line_indent + tag_indent;
                run.count += 1;
                i += 1;
                continue;
            }
        }

        // Mesh extents.
        if info[n].visible {
            run.extents[0] = run.extents[0].min(info[n].bl[0]);
            run.extents[1] = run.extents[1].min(info[n].bl[1]);
            run.extents[2] = run.extents[2].max(info[n].tr[0]);
            run.extents[3] = run.extents[3].max(info[n].tr[1]);
        }

        // Word boundaries.
        if style.word_wrapping || style.overflow == 1 || style.overflow == 3 {
            let breaking = (is_whitespace(c) || c == 0x200B || c == 0x2D || c == 0xAD)
                && !matches!(c, 0xA0 | 0x2007 | 0x2011 | 0x202F | 0x2060);
            if breaking {
                wrap = Saved { index: i as isize, run };
                char_wrapping = false;
                first_word = false;
            } else if is_cjk(c) {
                if last_breaking || first_word {
                    wrap = Saved { index: i as isize, run };
                    char_wrapping = false;
                    first_word = false;
                } else {
                    log::warn!("line breaking rules for U+{c:04X} are not supported yet");
                }
            } else if first_word || char_wrapping || last_breaking {
                wrap = Saved { index: i as isize, run };
            }
        }
        run.count += 1;
        i += 1;
    }
    let count = run.count;

    // Anchor of the block in the rectangle.
    let corners = [[rx, ry], [rx, ry + rh], [rx + rw, ry + rh], [rx + rw, ry]];
    let m = style.margin;
    let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let anchor = match style.alignment & 0xFF00 {
        align::TOP => [corners[1][0] + m[0], corners[1][1] - run.max_ascender - m[1]],
        align::MIDDLE => {
            let c = mid(corners[0], corners[1]);
            [c[0] + m[0], c[1] - (run.max_ascender + m[1] + max_visible_descender - m[3]) / 2.0]
        }
        align::BOTTOM => [corners[0][0] + m[0], corners[0][1] - max_visible_descender + m[3]],
        align::BASELINE => {
            let c = mid(corners[0], corners[1]);
            [c[0] + m[0], c[1]]
        }
        align::MIDLINE => {
            let c = mid(corners[0], corners[1]);
            [c[0] + m[0], c[1] - (run.extents[3] + m[1] + run.extents[1] - m[3]) / 2.0]
        }
        align::CAPLINE => {
            let c = mid(corners[0], corners[1]);
            [c[0] + m[0], c[1] - (run.max_cap_height - m[1] - m[3]) / 2.0]
        }
        _ => [0.0, 0.0],
    };

    // Justify each line and emit the visible glyphs.
    let mut out = Vec::new();
    for ch in info.iter().take(count) {
        if !ch.visible {
            continue;
        }
        let line = &lines[ch.line];
        let just = match line.alignment & 0xFF {
            align::LEFT => line.margin_left,
            align::CENTER => line.margin_left + line.width / 2.0 - line.max_advance / 2.0,
            align::RIGHT => line.margin_left + line.width - line.max_advance,
            align::GEOMETRY_CENTER => {
                line.margin_left + line.width / 2.0 - (line.extents[0] + line.extents[2]) / 2.0
            }
            align::JUSTIFIED | align::FLUSH => {
                log::warn!("justified text is not supported yet");
                line.margin_left
            }
            _ => 0.0,
        };
        let off = [anchor[0] + just, anchor[1]];
        let p = |v: [f32; 2]| [v[0] + off[0], v[1] + off[1]];
        out.push(GlyphQuad {
            material: ch.material.map(|m| materials[m].0.clone()),
            pos: [p(ch.bl), p(ch.tl), p(ch.tr), p(ch.br)],
            uv: ch.uv,
            scale: ch.scale * scale_ratio.abs(),
            colors: ch.colors,
            atlas: ch.atlas,
        });
    }
    out
}

/// Split markup from text: the characters to lay out, each one's material (index into the returned list of
/// tag materials, `None` for the text's own) and the tag materials (name, metrics) in order of first use.
#[allow(clippy::type_complexity)]
fn parse_material_tags(
    text: &str,
    rich_text: bool,
    tag_material: &dyn Fn(&str) -> Option<MaterialMetrics>,
) -> (Vec<u32>, Vec<Option<usize>>, Vec<(String, MaterialMetrics)>) {
    let src: Vec<char> = text.chars().collect();
    let mut chars = Vec::with_capacity(src.len());
    let mut char_material = Vec::with_capacity(src.len());
    let mut materials: Vec<(String, MaterialMetrics)> = Vec::new();
    let mut stack: Vec<Option<usize>> = vec![None];
    let mut i = 0;
    while i < src.len() {
        if rich_text
            && src[i] == '<'
            && let Some(end) = src[i..].iter().position(|&c| c == '>').map(|e| i + e)
        {
            let tag: String = src[i + 1..end].iter().collect();
            let lower = tag.to_ascii_lowercase();
            if lower == "/material" {
                if stack.len() > 1 {
                    stack.pop();
                }
                i = end + 1;
                continue;
            }
            if let Some(value) =
                lower.strip_prefix("material=").map(|_| tag["material=".len()..].trim_matches('"'))
            {
                if value.eq_ignore_ascii_case("default") {
                    stack.push(None);
                    i = end + 1;
                    continue;
                }
                let found = materials.iter().position(|(n, _)| n == value).or_else(|| {
                    tag_material(value).map(|m| {
                        materials.push((value.to_string(), m));
                        materials.len() - 1
                    })
                });
                if let Some(idx) = found {
                    stack.push(Some(idx));
                    i = end + 1;
                    continue;
                }
            } else {
                log::warn!("text tag <{tag}> is not supported yet");
            }
        }
        if src[i] == '\\' {
            log::warn!("escaped control characters are not supported yet: {text:?}");
        }
        chars.push(u32::from(src[i]));
        char_material.push(*stack.last().unwrap_or(&None));
        i += 1;
    }
    (chars, char_material, materials)
}

/// Close the current line: its first and last visible characters and its extents.
fn close_line(line: &mut LineInfo, info: &[CharInfo], run: &mut Run, max_visible_descender: &mut f32) {
    let line_ascender = run.max_line_ascender - run.line_offset;
    let line_descender = run.max_line_descender - run.line_offset;
    run.max_descender = run.max_descender.min(line_descender);
    *max_visible_descender = run.max_descender;
    if run.first_of_line > run.first_visible_of_line {
        run.first_visible_of_line = run.first_of_line;
    }
    if run.last_visible_of_line < run.first_visible_of_line {
        run.last_visible_of_line = run.first_visible_of_line;
    }
    line.extents = [
        info[run.first_visible_of_line].bl[0],
        line_descender,
        info[run.last_visible_of_line].tr[0],
        line_ascender,
    ];
}

/// Move characters `first..=last` down by `offset`.
fn shift_line(info: &mut [CharInfo], first: usize, last: usize, offset: f32) {
    for c in info.iter_mut().take(last + 1).skip(first) {
        for v in [&mut c.bl, &mut c.tl, &mut c.tr, &mut c.br] {
            v[1] -= offset;
        }
        c.ascender -= offset;
        c.baseline -= offset;
        c.descender -= offset;
    }
}

#[allow(clippy::too_many_arguments)]
fn save_glyph(
    ch: &mut CharInfo,
    glyph: &Glyph,
    font: &FontAsset,
    padding: f32,
    style_padding: f32,
    is_sdf: bool,
    vertex_color: Color32,
    font_color: Color32,
    style: &TextStyle,
) {
    let mut vc = vertex_color;
    vc.0[3] = vc.0[3].min(font_color.0[3]);
    ch.colors = match style.vertex_gradient {
        // Gradient corners multiply the vertex color: top-left, top-right, bottom-left, bottom-right.
        Some([tl, tr, bl, br]) => {
            let mul = |g: Color| {
                let v = vc.to_f32();
                Color32::from(Color::new(v[0] * g.r, v[1] * g.g, v[2] * g.b, v[3] * g.a))
            };
            [mul(bl), mul(tl), mul(tr), mul(br)]
        }
        None => [vc; 4],
    };
    let sp = if is_sdf { style_padding } else { 0.0 };
    let (aw, ah) = (font.atlas_size.0 as f32, font.atlas_size.1 as f32);
    let [x, y, w, h] = glyph.rect.map(|v| v as f32);
    let x0 = (x - padding - sp) / aw;
    let y0 = (y - padding - sp) / ah;
    let x1 = (x + padding + sp + w) / aw;
    let y1 = (y + padding + sp + h) / ah;
    ch.uv = [[x0, y0], [x0, y1], [x1, y1], [x1, y0]];
    ch.atlas = glyph.atlas;
}

fn lines_justified(alignment: u32) -> bool {
    alignment & align::FLUSH != 0 || alignment & align::JUSTIFIED != 0
}

fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < 0.000001
}

/// White space as the .NET runtime classifies it.
fn is_whitespace(c: u32) -> bool {
    matches!(c, 0x09..=0x0D | 0x20 | 0x85 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000)
}

fn is_cjk(c: u32) -> bool {
    (c > 0x1100 && c < 0x11FF)
        || (c > 0x2E80 && c < 0x9FFF)
        || (c > 0xA960 && c < 0xA97F)
        || (c > 0xAC00 && c < 0xD7FF)
        || (c > 0xF900 && c < 0xFAFF)
        || (c > 0xFE30 && c < 0xFE4F)
        || (c > 0xFF00 && c < 0xFFEF)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::font::{Character, FaceInfo};

    /// Point size 10 (scale 1 at size 10): ascent 8, descent -2, line height 14 (gap 4).
    fn font() -> FontAsset {
        let glyph = |w: f32, h: f32, bx: f32, by: f32, adv: f32, rect: [i32; 4]| Glyph {
            width: w,
            height: h,
            bearing_x: bx,
            bearing_y: by,
            advance: adv,
            rect,
            scale: 1.0,
            atlas: 0,
        };
        let mut glyphs = HashMap::new();
        glyphs.insert(1, glyph(4.0, 6.0, 1.0, 6.0, 6.0, [10, 20, 4, 6]));
        glyphs.insert(2, glyph(0.0, 0.0, 0.0, 0.0, 3.0, [0, 0, 0, 0]));
        glyphs.insert(3, glyph(0.0, 0.0, 0.0, 0.0, 0.0, [0, 0, 0, 0]));
        let mut characters = HashMap::new();
        characters.insert('a' as u32, Character { glyph: 1, scale: 1.0 });
        characters.insert(' ' as u32, Character { glyph: 2, scale: 1.0 });
        characters.insert('\n' as u32, Character { glyph: 3, scale: 1.0 });
        FontAsset {
            name: "test".into(),
            face: FaceInfo {
                point_size: 10.0,
                scale: 1.0,
                line_height: 14.0,
                ascent_line: 8.0,
                cap_line: 7.0,
                descent_line: -2.0,
                ..FaceInfo::default()
            },
            glyphs,
            characters,
            atlases: Vec::new(),
            material: None,
            atlas_size: (100, 100),
            padding: 0.0,
            normal_style: 0.0,
            normal_spacing: 0.0,
            bold_style: 0.75,
            bold_spacing: 7.0,
            italic_style: 35.0,
            tab_size: 10.0,
        }
    }

    const PLAIN: MaterialMetrics = MaterialMetrics { padding: 0.0, is_sdf: false, gradient: None };

    fn style(alignment: u32) -> TextStyle {
        TextStyle { font_size: 10.0, alignment, ..TextStyle::default() }
    }

    #[test]
    fn one_line_from_the_top_left() {
        let q = layout(
            "aa",
            &font(),
            PLAIN,
            &|_| None,
            &style(align::TOP | align::LEFT),
            [0.0, -20.0, 100.0, 20.0],
            1.0,
        );
        assert_eq!(q.len(), 2);
        // Bearing 1, glyph 4x6 on the baseline, moved down by the ascender (8).
        assert_eq!(q[0].pos, [[1.0, -8.0], [1.0, -2.0], [5.0, -2.0], [5.0, -8.0]]);
        assert_eq!(q[1].pos[0], [7.0, -8.0]);
        assert_eq!(q[0].uv, [[0.1, 0.2], [0.1, 0.26], [0.14, 0.26], [0.14, 0.2]]);
        assert_eq!(q[0].scale, 1.0);
    }

    #[test]
    fn words_wrap_at_the_last_space() {
        let s = style(align::TOP | align::LEFT);
        let q = layout("aa aa", &font(), PLAIN, &|_| None, &s, [0.0, -40.0, 15.0, 40.0], 1.0);
        assert_eq!(q.len(), 4);
        // Second line: down by -descender 2 + ascender 8 + gap 4.
        assert_eq!(q[2].pos[0], [1.0, -8.0 - 14.0]);
        assert_eq!(q[3].pos[0], [7.0, -8.0 - 14.0]);
    }

    #[test]
    fn line_feeds_start_new_lines() {
        let s = style(align::TOP | align::LEFT);
        let q = layout("a\na", &font(), PLAIN, &|_| None, &s, [0.0, -40.0, 100.0, 40.0], 1.0);
        assert_eq!(q.len(), 2);
        assert_eq!(q[1].pos[0], [1.0, -8.0 - 14.0]);
    }

    #[test]
    fn material_tags_switch_the_glyph_material() {
        let tagged = MaterialMetrics { padding: 1.0, is_sdf: false, gradient: None };
        let lookup = |name: &str| (name == "TMC15").then_some(tagged);
        let s = style(align::TOP | align::LEFT);
        let q = layout(
            "a<material=\"TMC15\">a</material>a",
            &font(),
            PLAIN,
            &lookup,
            &s,
            [0.0, -20.0, 100.0, 20.0],
            1.0,
        );
        assert_eq!(q.len(), 3);
        assert_eq!(q[0].material, None);
        assert_eq!(q[1].material.as_deref(), Some("TMC15"));
        assert_eq!(q[2].material, None);
        // The tagged glyph is padded by its material: one pixel more on every side.
        assert_eq!(q[1].pos[0], [7.0 - 1.0, -8.0 - 1.0]);
        // Unknown materials leave the tag in the text.
        let q = layout("<material=\"X\">", &font(), PLAIN, &lookup, &s, [0.0, -20.0, 100.0, 20.0], 1.0);
        // Only its two 'a's are in the test font.
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn middle_center_alignment() {
        let s = style(align::MIDDLE | align::CENTER);
        let q = layout("aa", &font(), PLAIN, &|_| None, &s, [0.0, 0.0, 100.0, 20.0], 1.0);
        // Vertical: centre 10 minus (ascender 8 + descender -2) / 2. Horizontal: (width - advance 12) / 2.
        let dx = (100.0 + WIDTH_EPSILON) / 2.0 - 6.0;
        assert_eq!(q[0].pos[0], [1.0 + dx, 10.0 - 3.0]);
    }
}
