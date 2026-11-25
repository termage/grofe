//! How the grid's tiles are laid out and drawn: each a framed box with its
//! title in the top edge and the bottom lines of its pane inside, never a
//! cell outside its own rect.

use alacritty_terminal::vte::ansi::{Color, NamedColor};

use crate::server::borders::{PaneBox, PaneLines};
use crate::server::listdraw::fit_text;
use crate::server::pane::{CellFlags, SnapCell};
use crate::server::render::{self, Frame, Screen};
use crate::server::status::{Style, escape_hashes};
use crate::types::PaneId;

/// Cells between two tile columns.
const GAP: usize = 1;

/// The narrowest tile: its frame, a title of one cell between a space and
/// an edge cell each side, and a body cell.
const MIN_TILE: usize = 7;

/// How the tiles sit on a client: one width per column (stretched to fill
/// the screen), the tile height, and the tile rows that fit under the query
/// line.
pub(super) struct Layout {
    pub(super) cols: usize,
    widths: Vec<usize>,
    tile_h: usize,
    pub(super) rows: usize,
}

/// As many columns of `tile_width` as fit `size` with a gap between each,
/// then widened evenly to fill it, so no tile is cut at the right edge.
/// A tile is its frame round `preview_lines` lines.
pub(super) fn layout(size: (u16, u16), tile_width: u16, preview_lines: u16) -> Layout {
    let w = usize::from(size.0);
    let want = usize::from(tile_width).max(MIN_TILE);
    let cols = ((w + GAP) / (want + GAP)).max(1);
    let room = w.saturating_sub(GAP * (cols - 1));
    let widths = (0..cols)
        .map(|c| room / cols + usize::from(c < room % cols))
        .collect();
    let tile_h = usize::from(preview_lines) + 2;
    let avail = usize::from(size.1).saturating_sub(1);
    Layout {
        cols,
        widths,
        tile_h,
        rows: (avail / tile_h).max(1),
    }
}

impl Layout {
    /// The rect (x, y, w, h) of the `slot`th tile shown, counted from the
    /// first tile of the top row on screen.
    pub(super) fn rect(&self, slot: usize) -> (usize, usize, usize, usize) {
        let col = slot % self.cols;
        let x = self.widths[..col].iter().sum::<usize>() + col * GAP;
        let y = 1 + (slot / self.cols) * self.tile_h;
        (x, y, self.widths[col], self.tile_h)
    }
}

/// The frame's lines and styles: `pane-border-style` for a tile,
/// `pane-active-border-style` for the selected one.
pub(super) struct Look {
    pub(super) lines: PaneLines,
    pub(super) border: Style,
    pub(super) active: Style,
}

/// One tile: its title, its pane's number (`pane-border-lines number`
/// draws it) and the rows of its pane's screen.
pub(super) struct Tile {
    pub(super) label: String,
    pub(super) index: usize,
    pub(super) body: Vec<Vec<SnapCell>>,
}

pub(super) fn blank() -> SnapCell {
    SnapCell {
        c: ' ',
        fg: Color::Named(NamedColor::Foreground),
        bg: Color::Named(NamedColor::Background),
        flags: CellFlags::empty(),
        zerowidth: Vec::new(),
        underline_color: None,
        hyperlink: None,
    }
}

/// The tiles from `tiles[first]` on, as many as the layout shows, with
/// `selected` (an index into `tiles`) in the active style.
pub(super) fn draw_tiles(
    screen: &mut Screen,
    lay: &Layout,
    tiles: &[Tile],
    first: usize,
    selected: usize,
    look: &Look,
) {
    for (slot, (i, tile)) in tiles
        .iter()
        .enumerate()
        .skip(first)
        .take(lay.rows * lay.cols)
        .enumerate()
    {
        let style = if i == selected {
            look.active
        } else {
            look.border
        };
        draw_tile(screen, lay.rect(slot), tile, look.lines, style);
    }
}

fn draw_tile(
    screen: &mut Screen,
    (x, y, w, h): (usize, usize, usize, usize),
    tile: &Tile,
    lines: PaneLines,
    style: Style,
) {
    let (cols, rows) = (usize::from(screen.cols), usize::from(screen.rows));
    if w < 2 || h < 2 || x + w > cols {
        return;
    }
    // The bottom tile row may run past the screen: keep to what is on it.
    let h = h.min(rows.saturating_sub(y));
    if h < 2 {
        return;
    }
    let (iw, ih) = (w - 2, h - 2);
    for row in y + 1..y + 1 + ih {
        for col in x + 1..x + 1 + iw {
            screen.cells[row * cols + col] = blank();
        }
    }
    let content = PaneBox {
        id: PaneId(0),
        x: (x + 1) as i32,
        y: (y + 1) as i32,
        w: iw as i32,
        h: ih as i32,
        index: tile.index,
    };
    // The frame's title runs from two cells in, between a space each side,
    // and leaves the edge cell before the right corner: `w - 6` cells of
    // text.
    let title = (w > 6).then(|| escape_hashes(&fit_text(&tile.label, w - 6)));
    let frame = Frame {
        lines,
        style,
        title: title.as_deref(),
        tab: None,
        marked: false,
    };
    render::draw_frame(screen, &content, &frame);
    let from = tile.body.len().saturating_sub(ih);
    for (j, line) in tile.body[from..].iter().enumerate() {
        let at = (y + 1 + j) * cols + x + 1;
        for (i, cell) in line.iter().take(iw).enumerate() {
            let cut = (i + 1 == iw && cell.flags.contains(CellFlags::WIDE_CHAR))
                || (i == 0 && cell.flags.contains(CellFlags::WIDE_CHAR_SPACER));
            if !cut {
                screen.cells[at + i] = cell.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthChar;

    const CORNERS: [char; 4] = ['┌', '┐', '└', '┘'];

    fn look() -> Look {
        Look {
            lines: PaneLines::parse("single"),
            border: Style::default(),
            active: Style::default(),
        }
    }

    /// A pane row of `text`, wide characters as cell plus spacer.
    fn row(text: &str) -> Vec<SnapCell> {
        let mut out = Vec::new();
        for c in text.chars() {
            if UnicodeWidthChar::width(c) == Some(2) {
                out.push(SnapCell {
                    c,
                    flags: CellFlags::WIDE_CHAR,
                    ..blank()
                });
                out.push(SnapCell {
                    flags: CellFlags::WIDE_CHAR_SPACER,
                    ..blank()
                });
            } else {
                out.push(SnapCell { c, ..blank() });
            }
        }
        out
    }

    fn tile(label: &str, body: &[&str]) -> Tile {
        Tile {
            label: label.into(),
            index: 0,
            body: body.iter().map(|t| row(t)).collect(),
        }
    }

    /// A screen of `size` filled with a marker, the tiles drawn over it.
    fn draw(size: (u16, u16), lay: &Layout, tiles: &[Tile]) -> Screen {
        let mut screen = Screen::new(size.0, size.1);
        screen.cells.fill(SnapCell { c: '@', ..blank() });
        draw_tiles(&mut screen, lay, tiles, 0, 0, &look());
        screen
    }

    fn at(screen: &Screen, x: usize, y: usize) -> &SnapCell {
        &screen.cells[y * usize::from(screen.cols) + x]
    }

    fn line(screen: &Screen, y: usize, x0: usize, x1: usize) -> String {
        (x0..x1).map(|x| at(screen, x, y).c).collect()
    }

    #[test]
    fn columns_fit_the_screen() {
        let one = layout((100, 40), 60, 12);
        assert_eq!(one.cols, 1);
        assert_eq!(one.rect(0), (0, 1, 100, 14));
        let three = layout((200, 40), 60, 12);
        assert_eq!(three.cols, 3);
        for slot in 0..3 {
            let (x, _, w, _) = three.rect(slot);
            assert!(x + w <= 200, "tile {slot} inside the screen");
            assert!(w >= 60);
        }
        let (x, _, w, _) = three.rect(2);
        assert_eq!(x + w, 200, "the last column reaches the right edge");
        assert_eq!(three.rect(1).0, three.rect(0).2 + 1, "a one-cell gap");
        assert_eq!(three.rect(3).1, 1 + 14, "the next row under the first");
    }

    #[test]
    fn tiny_tile_width_still_frames_and_titles() {
        let size = (130, 10);
        let lay = layout(size, 1, 3);
        assert_eq!(lay.cols, (130 + 1) / (MIN_TILE + 1));
        let tiles: Vec<Tile> = (0..lay.cols)
            .map(|i| tile(&format!("t{i} long"), &["body"]))
            .collect();
        let screen = draw(size, &lay, &tiles);
        for slot in 0..tiles.len() {
            let (x, y, w, h) = lay.rect(slot);
            assert!(w >= MIN_TILE && x + w <= 130);
            assert_eq!(at(&screen, x, y).c, CORNERS[0], "tile {slot}");
            assert_eq!(at(&screen, x + w - 1, y + h - 1).c, CORNERS[3]);
            let title = at(&screen, x + 3, y).c;
            assert!(
                title == 't' || title == '…',
                "tile {slot} has a title: {title}"
            );
            assert_eq!(at(&screen, x + 1, y + 1).c, 'b', "and a body");
        }
    }

    #[test]
    fn nothing_drawn_outside_a_tile() {
        let size = (130, 40);
        let lay = layout(size, 40, 5);
        let long = "x".repeat(300);
        let tiles: Vec<Tile> = (0..7)
            .map(|i| {
                tile(
                    &format!("tile {i} {long}"),
                    &[&long, "━".repeat(200).as_str()],
                )
            })
            .collect();
        let screen = draw(size, &lay, &tiles);
        let rects: Vec<_> = (0..tiles.len()).map(|s| lay.rect(s)).collect();
        for y in 0..usize::from(size.1) {
            for x in 0..usize::from(size.0) {
                let inside = rects
                    .iter()
                    .any(|&(rx, ry, w, h)| (rx..rx + w).contains(&x) && (ry..ry + h).contains(&y));
                if !inside {
                    assert_eq!(
                        at(&screen, x, y).c,
                        '@',
                        "cell ({x}, {y}) is outside every tile"
                    );
                }
            }
        }
    }

    #[test]
    fn every_tile_edge_is_border() {
        let size = (130, 40);
        let lay = layout(size, 40, 3);
        let tiles: Vec<Tile> = (0..5).map(|i| tile(&format!("t{i}"), &["body"])).collect();
        let screen = draw(size, &lay, &tiles);
        for slot in 0..tiles.len() {
            let (x, y, w, h) = lay.rect(slot);
            let (x1, y1) = (x + w - 1, y + h - 1);
            assert_eq!(at(&screen, x, y).c, CORNERS[0]);
            assert_eq!(at(&screen, x1, y).c, CORNERS[1]);
            assert_eq!(at(&screen, x, y1).c, CORNERS[2]);
            assert_eq!(at(&screen, x1, y1).c, CORNERS[3]);
            for yy in y + 1..y1 {
                assert_eq!(at(&screen, x, yy).c, '│');
                assert_eq!(at(&screen, x1, yy).c, '│');
            }
            assert_eq!(line(&screen, y1, x + 1, x1), "─".repeat(w - 2));
            assert_eq!(
                at(&screen, x + 1, y).c,
                '─',
                "the top edge before the title"
            );
            assert_eq!(
                at(&screen, x1 - 1, y).c,
                '─',
                "the top edge after the title"
            );
        }
    }

    #[test]
    fn long_title_ends_in_ellipsis() {
        let size = (40, 10);
        let lay = layout(size, 40, 3);
        let screen = draw(size, &lay, &[tile(&"agent ".repeat(20), &[])]);
        let top = line(&screen, 1, 0, 40);
        assert!(top.starts_with("┌─ agent"), "{top}");
        assert!(top.ends_with("… ─┐"), "{top}");
    }

    #[test]
    fn body_lines_clip_to_the_inner_width() {
        // An inner width of 7 (odd) under a row of CJK, two cells each.
        let size = (9, 6);
        let lay = layout(size, 9, 3);
        let long = "y".repeat(300);
        let screen = draw(size, &lay, &[tile("t", &[&long, "日本語漢字", "ab"])]);
        assert_eq!(line(&screen, 2, 0, 9), format!("│{}│", "y".repeat(7)));
        let cjk = at(&screen, 7, 3);
        assert_eq!(
            cjk.c, ' ',
            "the CJK character that would straddle the edge is dropped"
        );
        assert!(!cjk.flags.contains(CellFlags::WIDE_CHAR));
        assert_eq!(line(&screen, 3, 1, 7).replace(' ', ""), "日本語");
        assert_eq!(at(&screen, 8, 3).c, '│');
        assert_eq!(line(&screen, 4, 0, 9), "│ab     │");
    }
}
