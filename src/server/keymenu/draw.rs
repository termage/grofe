use alacritty_terminal::vte::ansi::{Color, NamedColor};

use super::{Row, active_pane, rows_for, slash_bound, visible};
use crate::server::State;
use crate::server::dispatch::options;
use crate::server::listdraw::{self, fill, fit_text, put_text, width};
use crate::server::listview::{self, ListState};
use crate::server::pane::CellFlags;
use crate::server::render::{Overlay, OverlayCtx, Screen};
use crate::server::status::{self, Style};
use crate::types::{ClientId, Rect};

/// The narrowest a column of bindings is laid out.
const MIN_COL: usize = 32;
const GAP: usize = 2;
/// The key column is as wide as the widest key, up to this.
const KEY_CAP: usize = 10;
/// Columns are balanced, but never shorter than this.
const MIN_HEIGHT: usize = 8;

pub(super) const HINT: &str = "type a key to run · / search · ↑↓ move · Esc close";
const HINT_C_F: &str = "type a key to run · C-f search · ↑↓ move · Esc close";
const SEARCH_HINT: &str = "Enter run · ↑↓ move · Esc back";

/// The largest box: 70% of a wide client, nearly all of a narrow one, so
/// a description is rarely cut.
pub(super) fn area(size: (u16, u16)) -> Rect {
    let base = listdraw::centred(size, 70, 70);
    let cols = u32::from(size.0);
    let wide = (cols * 70 / 100).max(cols.saturating_sub(2).min(100));
    let cols = u16::try_from(wide).unwrap_or(size.0);
    Rect {
        x: (size.0 - cols) / 2,
        cols,
        ..base
    }
}

/// Rows of bindings the largest box holds per column: inside the border,
/// minus the search line while it is shown.
pub(super) fn list_height(size: (u16, u16), filtering: bool) -> usize {
    usize::from(area(size).rows.saturating_sub(2 + u16::from(filtering)))
}

/// How many columns of at least [`MIN_COL`] cells fit `inner` cells.
fn columns(inner: usize) -> usize {
    ((inner + GAP) / (MIN_COL + GAP)).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Blank,
    /// The header of the group of this row.
    Head(usize),
    Item(usize),
}

/// `rows` in columns of at most `height` lines, each group under its
/// header and a blank line between groups. A header never ends a column.
fn place(rows: &[Row], height: usize) -> Vec<Vec<Line>> {
    let height = height.max(2);
    let mut cols = Vec::new();
    let mut col: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let head = i == 0 || rows[i - 1].group != row.group;
        let need = match (head, col.is_empty()) {
            (false, _) => 1,
            (true, true) => 2,
            (true, false) => 3,
        };
        if !col.is_empty() && col.len() + need > height {
            cols.push(std::mem::take(&mut col));
        }
        if head {
            if !col.is_empty() {
                col.push(Line::Blank);
            }
            col.push(Line::Head(i));
        }
        col.push(Line::Item(i));
    }
    cols.push(col);
    cols
}

/// The shortest balanced layout in `ncols` columns of at most `max_h`
/// lines; more columns than that are pages.
fn layout(rows: &[Row], ncols: usize, max_h: usize) -> Vec<Vec<Line>> {
    (MIN_HEIGHT.min(max_h)..max_h)
        .map(|h| place(rows, h))
        .find(|cols| cols.len() <= ncols)
        .unwrap_or_else(|| place(rows, max_h))
}

/// The client's styles: `mode-style` for the selected row, its colour for
/// group headers.
pub(super) struct Look {
    pub mode: Style,
}

impl Look {
    fn head(&self) -> Style {
        let mut head = Style::default();
        if self.mode.bg != Color::Named(NamedColor::Background) {
            head.fg = self.mode.bg;
        }
        head.attrs |= CellFlags::BOLD;
        head
    }
}

fn with(mut style: Style, flags: CellFlags) -> Style {
    style.attrs |= flags;
    style
}

/// What the menu shows.
pub(super) struct View<'a> {
    pub table: &'a str,
    pub rows: &'a [Row],
    pub ui: &'a ListState,
    /// `/` runs a binding, so only `C-f` searches.
    pub slash_bound: bool,
}

/// Draws the menu centred on `screen`: groups in columns, or the search
/// results while searching.
pub(super) fn render(screen: &mut Screen, view: &View, look: &Look) {
    let full = area((screen.cols, screen.rows));
    let kw = view
        .rows
        .iter()
        .map(|r| width(&r.key.to_string()))
        .max()
        .unwrap_or(0)
        .min(KEY_CAP);
    let title = format!("Keys · {}", view.table);
    if view.ui.filtering {
        draw_search(screen, full, &title, view, look, kw);
    } else {
        draw_groups(screen, full, title, view, look, kw);
    }
}

fn draw_groups(screen: &mut Screen, full: Rect, title: String, v: &View, look: &Look, kw: usize) {
    let inner = usize::from(full.cols).saturating_sub(4);
    let ncols = columns(inner);
    let colw = inner.saturating_sub(GAP * (ncols - 1)) / ncols;
    let max_h = usize::from(full.rows).saturating_sub(2).max(1);
    let cols = layout(v.rows, ncols, max_h);
    let sel = v.ui.selected.min(v.rows.len().saturating_sub(1));
    let at = cols
        .iter()
        .position(|c| c.contains(&Line::Item(sel)))
        .unwrap_or(0);
    let (page, pages) = (at / ncols, cols.len().div_ceil(ncols));
    let shown = &cols[page * ncols..cols.len().min(page * ncols + ncols)];
    let used = shown.len().max(1);
    let height = shown.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let hint = if v.slash_bound { HINT_C_F } else { HINT };
    let w = (used * colw + GAP * (used - 1) + 4).max(width(hint) + 6);
    let w = w.min(usize::from(full.cols));
    let h = (height + 2).min(usize::from(full.rows));
    let (w, h) = (w as u16, h as u16);
    let area = Rect {
        x: (screen.cols - w) / 2,
        y: (screen.rows - h) / 2,
        cols: w,
        rows: h,
    };
    let title = match pages {
        0 | 1 => title,
        _ => format!("{title} ({}/{pages})", page + 1),
    };
    if !listdraw::draw_frame(screen, area, &title) {
        return;
    }
    let bottom = usize::from(area.y + area.rows) - 1;
    let right = usize::from(area.x + area.cols) - 1;
    for (c, col) in shown.iter().enumerate() {
        let x = usize::from(area.x) + 2 + c * (colw + GAP);
        let end = (x + colw).min(right);
        for (n, line) in col.iter().enumerate() {
            let y = usize::from(area.y) + 1 + n;
            if y >= bottom || x >= end {
                break;
            }
            match *line {
                Line::Blank => {}
                Line::Head(i) => {
                    let group = fit_text(&v.rows[i].group, end - x);
                    put_text(screen, x, y, end, &group, &look.head());
                }
                Line::Item(i) => {
                    let item = Item {
                        row: &v.rows[i],
                        selected: i == sel,
                        hits: &[],
                        tail: false,
                    };
                    draw_item(screen, (x, end, y), kw, &item, look);
                }
            }
        }
    }
    draw_hint(screen, area, hint);
}

fn draw_search(screen: &mut Screen, area: Rect, title: &str, v: &View, look: &Look, kw: usize) {
    if !listdraw::draw_frame(screen, area, title) || area.cols < 5 || area.rows < 3 {
        return;
    }
    let (x, y0) = (usize::from(area.x) + 2, usize::from(area.y));
    let end = usize::from(area.x + area.cols) - 2;
    let bottom = y0 + usize::from(area.rows) - 1;
    let after = put_text(screen, x, y0 + 1, end, "/ ", &look.head());
    let after = put_text(screen, after, y0 + 1, end, &v.ui.query, &Style::default());
    let dim = with(Style::default(), CellFlags::DIM);
    let shown = visible(v.rows, &v.ui.query);
    if v.ui.query.is_empty() {
        put_text(screen, after, y0 + 1, end, "type to search", &dim);
    } else if shown.is_empty() {
        put_text(screen, x, y0 + 2, end, "no match", &dim);
    }
    let height = bottom.saturating_sub(y0 + 2);
    let mut ui = v.ui.clone();
    ui.clamp(shown.len(), height);
    for (n, &i) in shown.iter().skip(ui.top).take(height).enumerate() {
        let row = &v.rows[i];
        let hits = listview::matches(&v.ui.query, &row.haystack())
            .map(|(_, hits)| hits)
            .unwrap_or_default();
        let item = Item {
            row,
            selected: ui.top + n == ui.selected,
            hits: &hits,
            tail: true,
        };
        draw_item(screen, (x, end, y0 + 2 + n), kw, &item, look);
    }
    draw_hint(screen, area, SEARCH_HINT);
}

/// One binding to draw. `hits` are matched chars of the row's haystack;
/// `tail` adds its group.
struct Item<'a> {
    row: &'a Row,
    selected: bool,
    hits: &'a [usize],
    tail: bool,
}

/// `key  label  · group` in cells `x..end` of row `y`: the key bold and
/// right-aligned in `kw` cells, a label that is only a command dimmed.
fn draw_item(
    screen: &mut Screen,
    (x, end, y): (usize, usize, usize),
    kw: usize,
    it: &Item,
    look: &Look,
) {
    let base = if it.selected {
        look.mode
    } else {
        Style::default()
    };
    if it.selected {
        fill(screen, x, end, y, &base);
    }
    let mark = with(base, CellFlags::BOLD | CellFlags::UNDERLINE);
    let key = it.row.key.to_string();
    let shown = fit_text(&key, kw.min(end - x));
    let pad = kw.saturating_sub(width(&shown));
    let key_style = with(base, CellFlags::BOLD);
    put_marked(
        screen,
        (x + pad, end, y),
        &key,
        &shown,
        0,
        it.hits,
        key_style,
        mark,
    );
    let x = x + kw + 2;
    if x >= end {
        return;
    }
    let room = end - x;
    let label_off = key.chars().count() + 1;
    let tail_w = width(&it.row.group) + 4;
    let tail = it.tail && room >= tail_w + 8;
    let label_room = if tail { room - tail_w } else { room };
    let label = fit_text(&it.row.label, label_room);
    let label_style = if it.row.described {
        base
    } else {
        with(base, CellFlags::DIM)
    };
    let x = put_marked(
        screen,
        (x, end, y),
        &it.row.label,
        &label,
        label_off,
        it.hits,
        label_style,
        mark,
    );
    if tail {
        let dim = with(base, CellFlags::DIM);
        let x = put_text(screen, x, y, end, "  · ", &dim);
        let group_off = label_off + it.row.label.chars().count() + 1;
        let group = &it.row.group;
        put_marked(
            screen,
            (x, end, y),
            group,
            group,
            group_off,
            it.hits,
            dim,
            mark,
        );
    }
}

/// Draws `shown` (`orig` as cut to fit) with the chars whose haystack
/// position, `offset` on, is in `hits` in `mark`. Returns the next column.
#[allow(clippy::too_many_arguments)]
fn put_marked(
    screen: &mut Screen,
    (mut x, end, y): (usize, usize, usize),
    orig: &str,
    shown: &str,
    offset: usize,
    hits: &[usize],
    style: Style,
    mark: Style,
) -> usize {
    let orig: Vec<char> = orig.chars().collect();
    for (i, c) in shown.chars().enumerate() {
        let hit = orig.get(i) == Some(&c) && hits.contains(&(offset + i));
        let st = if hit { mark } else { style };
        x = put_text(screen, x, y, end, c.encode_utf8(&mut [0; 4]), &st);
    }
    x
}

/// The dim key hint in the box's bottom edge.
fn draw_hint(screen: &mut Screen, area: Rect, hint: &str) {
    if area.cols <= 6 || area.rows <= 2 {
        return;
    }
    let dim = with(Style::default(), CellFlags::DIM);
    let hint = format!(" {} ", fit_text(hint, usize::from(area.cols) - 6));
    let (x, y) = (usize::from(area.x) + 2, usize::from(area.y + area.rows) - 1);
    let end = usize::from(area.x + area.cols) - 1;
    put_text(screen, x, y, end, &hint, &dim);
}

/// The client's look, from its active pane's options.
fn look(state: &State, client: ClientId) -> Look {
    let chain = state
        .clients
        .get(&client)
        .and_then(|c| active_pane(state, c.session))
        .map(|p| options::chain(&state.model, p))
        .unwrap_or_default();
    let mode = state.model.options.lookup("mode-style", &chain);
    Look {
        mode: status::parse_style(mode.unwrap_or_default()),
    }
}

/// Draws a client's key menu.
pub struct KeyMenuOverlay;

impl Overlay for KeyMenuOverlay {
    fn draw(&self, state: &State, client: ClientId, _cx: &OverlayCtx, screen: &mut Screen) {
        let Some(menu) = state.keymenus.open.get(&client) else {
            return;
        };
        let rows = rows_for(state, client, &menu.table);
        let view = View {
            table: &menu.table,
            rows: &rows,
            ui: &menu.ui,
            slash_bound: slash_bound(&state.registry.key_tables, &menu.table),
        };
        render(screen, &view, &look(state, client));
        screen.cursor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::input::parse_key;
    use crate::server::keymenu::is_reserved;

    fn row(key: &str, group: &str, desc: Option<&str>, cmd: &str) -> Row {
        Row {
            key: parse_key(key).expect("key"),
            group: group.into(),
            label: desc.unwrap_or(cmd).into(),
            cmd: cmd.into(),
            described: desc.is_some(),
        }
    }

    /// `n` bindings in groups of 8, keys `F1`.. and `M-a`.. as needed.
    fn many(n: usize) -> Vec<Row> {
        (0..n)
            .map(|i| {
                let key = format!("M-{}{}", char::from(b'a' + (i % 26) as u8), i / 26);
                let key = if parse_key(&key).is_ok() {
                    key
                } else {
                    "M-a".into()
                };
                row(
                    &key,
                    &format!("g{}", i / 8),
                    Some(&format!("desc {i}")),
                    "x",
                )
            })
            .collect()
    }

    fn look() -> Look {
        Look {
            mode: status::parse_style("bg=yellow,fg=black"),
        }
    }

    fn draw(size: (u16, u16), rows: &[Row], ui: &ListState) -> Screen {
        let mut screen = Screen::new(size.0, size.1);
        for cell in &mut screen.cells {
            cell.c = 'X';
        }
        let view = View {
            table: "prefix",
            rows,
            ui,
            slash_bound: false,
        };
        render(&mut screen, &view, &look());
        screen
    }

    fn line(s: &Screen, y: usize) -> String {
        let cols = usize::from(s.cols);
        s.cells[y * cols..(y + 1) * cols]
            .iter()
            .filter(|c| !c.flags.contains(CellFlags::WIDE_CHAR_SPACER))
            .map(|c| c.c)
            .collect()
    }

    fn lines(s: &Screen) -> Vec<String> {
        (0..usize::from(s.rows)).map(|y| line(s, y)).collect()
    }

    /// The first cells that read `text`.
    fn run_of<'a>(s: &'a Screen, text: &str) -> &'a [crate::server::pane::SnapCell] {
        let n = text.chars().count();
        s.cells
            .windows(n)
            .find(|w| w.iter().map(|c| c.c).collect::<String>() == text)
            .unwrap_or_else(|| panic!("{text:?} not drawn"))
    }

    /// The drawn box's corners, from its border glyphs.
    fn corners(s: &Screen) -> (usize, usize, usize, usize) {
        let cols = usize::from(s.cols);
        let at = |c: char| s.cells.iter().position(|x| x.c == c).expect("corner");
        let (tl, br) = (at('╭'), at('╯'));
        (tl % cols, tl / cols, br % cols, br / cols)
    }

    #[test]
    fn column_count_follows_the_width() {
        for (w, n) in [(60u16, 1), (120, 2), (200, 4)] {
            let inner = usize::from(area((w, 50)).cols) - 4;
            assert_eq!(columns(inner), n, "width {w}");
            let s = draw((w, 50), &many(60), &ListState::default());
            let (x0, y0, x1, _) = corners(&s);
            let heads = (x0..=x1)
                .filter(|&x| {
                    let cell = &s.cells[(y0 + 1) * usize::from(w) + x];
                    cell.c != ' '
                        && (x == x0 + 2 || s.cells[(y0 + 1) * usize::from(w) + x - 1].c == ' ')
                })
                .count();
            assert!(heads >= n, "width {w}: {:?}", line(&s, y0 + 1));
        }
    }

    #[test]
    fn group_headers_bold_and_in_order_other_last() {
        let rows = [
            row("c", "window", Some("New window"), "new-window"),
            row("n", "window", None, "next-window"),
            row("x", "pane", Some("Kill pane"), "kill-pane"),
            row("z", "other", None, "resize-pane -Z"),
        ];
        let heads: Vec<&str> = layout(&rows, 4, 20)
            .concat()
            .into_iter()
            .filter_map(|l| match l {
                Line::Head(i) => Some(rows[i].group.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(heads, ["window", "pane", "other"]);
        let s = draw((120, 40), &rows, &ListState::default());
        let text = lines(&s).join("\n");
        for head in ["window", "pane", "other"] {
            let cells = run_of(&s, &format!(" {head} "));
            assert!(
                cells[1].flags.contains(CellFlags::BOLD),
                "{head} bold: {text}"
            );
        }
        assert!(text.contains("New window"));
        let dimmed = run_of(&s, "next-window");
        assert!(
            dimmed[0].flags.contains(CellFlags::DIM),
            "no desc: command dimmed"
        );
        assert!(text.contains("╭─ Keys · prefix "), "{text}");
        assert!(text.contains(HINT), "{text}");
    }

    #[test]
    fn keys_right_aligned_in_a_bold_column() {
        let rows = [
            row("c", "g", Some("one"), "x"),
            row("M-Up", "g", Some("two"), "x"),
        ];
        let s = draw((60, 20), &rows, &ListState::default());
        let text = lines(&s);
        let one = text.iter().find(|l| l.contains("one")).expect("one");
        let two = text.iter().find(|l| l.contains("two")).expect("two");
        assert_eq!(one.find("c  one"), two.find("p  two"), "keys end together");
        assert_eq!(one.find("one"), two.find("two"));
    }

    #[test]
    fn nothing_drawn_outside_the_box() {
        let mut search = ListState {
            filtering: true,
            query: "d".into(),
            ..ListState::default()
        };
        for size in [(60, 20), (120, 40), (200, 50), (30, 8), (7, 4), (3, 3)] {
            for ui in [ListState::default(), search.clone()] {
                let s = draw(size, &many(90), &ui);
                let full = area(size);
                for (i, cell) in s.cells.iter().enumerate() {
                    let (x, y) = (i % usize::from(size.0), i / usize::from(size.0));
                    let inside = (usize::from(full.x)..usize::from(full.x + full.cols))
                        .contains(&x)
                        && (usize::from(full.y)..usize::from(full.y + full.rows)).contains(&y);
                    if !inside {
                        assert_eq!(cell.c, 'X', "{size:?} drew at {x},{y}");
                    }
                }
                if size.0 >= 30 {
                    let (x0, y0, x1, y1) = corners(&s);
                    for (i, cell) in s.cells.iter().enumerate() {
                        let (x, y) = (i % usize::from(size.0), i / usize::from(size.0));
                        let inside = (x0..=x1).contains(&x) && (y0..=y1).contains(&y);
                        assert_eq!(cell.c == 'X', !inside, "{size:?} at {x},{y}");
                    }
                }
            }
            search.selected = 80;
        }
    }

    #[test]
    fn long_description_ends_in_an_ellipsis() {
        let long = "a description far too long for any column of this box, it goes on";
        let rows = [row("c", "g", Some(long), "x")];
        let s = draw((60, 20), &rows, &ListState::default());
        let text = lines(&s);
        let l = text
            .iter()
            .find(|l| l.contains("a description"))
            .expect("row");
        let from = l.find("a description").expect("start");
        let cut = l[from..].split('…').next().expect("cut");
        assert!(l.contains('…'), "{l:?}");
        assert!(long.starts_with(cut) && cut.len() > 20, "{l:?}");
    }

    #[test]
    fn wide_characters_never_straddle_a_column_edge() {
        let wide = "日本語".repeat(20);
        let rows: Vec<Row> = (0..30)
            .map(|i| row(&format!("F{}", i % 12 + 1), "g", Some(&wide), "x"))
            .collect();
        let size = (120u16, 40u16);
        let s = draw(size, &rows, &ListState::default());
        let (x0, y0, x1, y1) = corners(&s);
        let cols = usize::from(size.0);
        let colw = (x1 - x0 - 3 - GAP) / 2;
        let edges = [x0 + 2 + colw, x0 + 2 + colw + GAP + colw];
        for y in y0 + 1..y1 {
            for e in edges {
                for x in e..(e + GAP).min(x1 + 1) {
                    let cell = &s.cells[y * cols + x];
                    assert!(
                        !cell
                            .flags
                            .intersects(CellFlags::WIDE_CHAR | CellFlags::WIDE_CHAR_SPACER),
                        "wide char across the edge at {x},{y}: {:?}",
                        line(&s, y)
                    );
                }
            }
        }
        assert!(
            lines(&s)
                .iter()
                .any(|l| ["日…", "本…", "語…"].iter().any(|t| l.contains(t))),
            "{:#?}",
            lines(&s)
        );
    }

    #[test]
    fn selected_row_stays_visible_when_the_table_scrolls() {
        let rows = many(200);
        for filtering in [false, true] {
            for sel in [0, 57, 199] {
                let ui = ListState {
                    selected: sel,
                    filtering,
                    query: if filtering {
                        "desc".into()
                    } else {
                        String::new()
                    },
                    ..ListState::default()
                };
                let s = draw((60, 20), &rows, &ui);
                let want = if filtering {
                    let shown = visible(&rows, "desc");
                    rows[shown[sel]].label.clone()
                } else {
                    rows[sel].label.clone()
                };
                let text = lines(&s);
                let hit = text
                    .iter()
                    .position(|l| l.contains(&format!("{want} ")) || l.ends_with(&want))
                    .unwrap_or_else(|| panic!("{want} missing: {text:#?}"));
                let y = hit * usize::from(s.cols);
                assert!(
                    s.cells[y..y + usize::from(s.cols)]
                        .iter()
                        .any(|c| c.bg == look().mode.bg),
                    "{want} drawn selected"
                );
            }
        }
    }

    #[test]
    fn fuzzy_search_ranks_split_window_first_and_marks_the_match() {
        let rows = [
            row("y", "other", Some("show playlist tabs"), "x"),
            row("x", "pane", Some("Kill pane"), "kill-pane"),
            row("%", "pane", None, "split-window -h"),
        ];
        let shown = visible(&rows, "splt");
        assert_eq!(rows[shown[0]].cmd, "split-window -h", "{shown:?}");
        let ui = ListState {
            filtering: true,
            query: "splt".into(),
            ..ListState::default()
        };
        let s = draw((120, 30), &rows, &ui);
        let (_, y0, _, _) = corners(&s);
        let first = line(&s, y0 + 2);
        assert!(first.contains("%  split-window -h  · pane"), "{first:?}");
        assert!(line(&s, y0 + 1).contains("/ splt"));
        let cols = usize::from(s.cols);
        let marked: String = s.cells[(y0 + 2) * cols..(y0 + 3) * cols]
            .iter()
            .filter(|c| c.flags.contains(CellFlags::UNDERLINE))
            .map(|c| c.c)
            .collect();
        assert_eq!(marked, "splt");
        assert!(lines(&s).join("\n").contains(SEARCH_HINT));
    }

    #[test]
    fn hint_names_c_f_when_slash_is_bound() {
        assert!(!is_reserved(&parse_key("/").expect("key")));
        let rows = [row("/", "g", Some("Describe key"), "x")];
        let mut screen = Screen::new(80, 20);
        let view = View {
            table: "prefix",
            rows: &rows,
            ui: &ListState::default(),
            slash_bound: true,
        };
        render(&mut screen, &view, &look());
        assert!(lines(&screen).join("\n").contains(HINT_C_F));
    }
}
