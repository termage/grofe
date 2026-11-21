use super::*;

/// tmux's `DEFAULT_XPIXEL` and `DEFAULT_YPIXEL`: a window's cell size until
/// a client says its own.
const DEFAULT_CELL_PX: (u16, u16) = (16, 32);

impl ServerVars<'_> {
    /// The variables of the client the formats are for and its view of the
    /// window (issue 4): `None` for a name that is not one of them, or when
    /// tmux 3.7b has no value (no client, a window that fits).
    pub(in crate::server::dispatch::options) fn client_var(&self, name: &str) -> Option<String> {
        let model = &self.state.model;
        let window = model.panes.get(&self.pane)?.window;
        match name {
            // The clients whose session shows the window, in client order.
            "window_active_clients" | "window_active_clients_list" => {
                let names: Vec<&str> = self
                    .state
                    .clients
                    .values()
                    .filter(|c| model.sessions.get(&c.session).map(|s| s.active) == Some(window))
                    .map(|c| c.name.as_str())
                    .collect();
                match name {
                    "window_active_clients" => Some(names.len().to_string()),
                    _ => (!names.is_empty()).then(|| names.join(",")),
                }
            }
            // The size the window's panes were last given.
            "window_cell_width" | "window_cell_height" => {
                let active = model.windows.get(&window)?.active;
                let cell_px = model.panes.get(&active)?.handle.cell_px();
                let (w, h) = cell_px.unwrap_or(DEFAULT_CELL_PX);
                Some(if name == "window_cell_width" { w } else { h }.to_string())
            }
            // The status row under the mouse, on the status line only.
            "mouse_status_line" => {
                let m = self.state.mouse.current()?;
                m.range.as_ref()?;
                Some(m.pane_cell?.1.to_string())
            }
            "copy_cursor_line" | "copy_cursor_hyperlink" => {
                let cursor = |n| {
                    crate::server::copy::format_var(self.state, self.pane, n)?
                        .parse()
                        .ok()
                };
                let (x, y) = (cursor("copy_cursor_x")?, cursor("copy_cursor_y")?);
                let what = match name {
                    "copy_cursor_line" => "mouse_line",
                    _ => "mouse_hyperlink",
                };
                Some(
                    crate::server::copy::mouse_text(self.state, self.pane, x, y, what)
                        .unwrap_or_default(),
                )
            }
            _ => self.attached_var(name),
        }
    }

    /// The variables only a client has: whether it shows the formats'
    /// session, its cell size, and whether the window it shows is bigger
    /// than it and where its view of it starts.
    fn attached_var(&self, name: &str) -> Option<String> {
        let client_name = self.inner.vars().get("client_name")?;
        let (&id, c) = self
            .state
            .clients
            .iter()
            .find(|(_, c)| c.name == client_name)?;
        match name {
            "session_active" => Some(flag(Some(c.session) == self.session_id())),
            "client_created" => Some(epoch_secs(c.created)),
            "client_cell_width" | "client_cell_height" => {
                let (w, h) = c.caps.cell_px.unwrap_or((0, 0));
                Some(if name == "client_cell_width" { w } else { h }.to_string())
            }
            "window_bigger" => Some(flag(!clients::window_fits(self.state, id, c)?.1)),
            "window_offset_x" | "window_offset_y" => {
                let (x, y) = self.window_offset(id, c)?;
                Some(if name == "window_offset_x" { x } else { y }.to_string())
            }
            _ => None,
        }
    }

    /// Where client `c`'s view of its window starts when the window is
    /// bigger than it, as tmux reads it for the formats.
    fn window_offset(
        &self,
        id: crate::types::ClientId,
        c: &clients::ClientEntry,
    ) -> Option<(u16, u16)> {
        let model = &self.state.model;
        let w = model.windows.get(&model.sessions.get(&c.session)?.active)?;
        let pan = c.pan.filter(|(window, ..)| *window == w.id);
        let pane = FormatCtx::new(model, w.active);
        let handle = &model.panes.get(&w.active)?.handle;
        let at = |origin: &str, cursor: &str| -> Option<u16> {
            let origin: u16 = pane.get(origin)?.parse().ok()?;
            let cursor: u16 = handle.term_var(cursor)?.parse().ok()?;
            Some(origin + cursor)
        };
        let cursor = match handle.term_var("cursor_flag").as_deref() {
            Some("1") => Some((at("pane_left", "cursor_x")?, at("pane_top", "cursor_y")?)),
            _ => None,
        };
        tmux_window_offset(
            clients::visible_size(self.state, id, c),
            (w.cols, w.rows),
            pan.map(|(_, x, y)| (x, y)),
            cursor,
        )
    }
}

/// tmux's `tty_window_offset1`: where a view of `visible` cells starts in a
/// window of `window` cells, `None` when the window fits. A panned view
/// stays where it was moved to, kept inside the window; otherwise the view
/// follows the cursor (window cells), from the near edge, clamped at the
/// far one, or with the cursor in the middle column and the bottom row.
/// grofe's own view ([`clients::follow`]) only moves as far as it must.
fn tmux_window_offset(
    visible: (u16, u16),
    window: (u16, u16),
    pan: Option<(u16, u16)>,
    cursor: Option<(u16, u16)>,
) -> Option<(u16, u16)> {
    let ((sx, sy), (wx, wy)) = (visible, window);
    if sx >= wx && sy >= wy {
        return None;
    }
    if let Some((x, y)) = pan {
        let keep = |at: u16, size: u16, whole: u16| match () {
            _ if size >= whole => 0,
            _ if at + size > whole => whole - size,
            _ => at,
        };
        return Some((keep(x, sx, wx), keep(y, sy, wy)));
    }
    let Some((cx, cy)) = cursor else {
        return Some((0, 0));
    };
    let ox = match () {
        _ if cx < sx => 0,
        _ if cx > wx.saturating_sub(sx) => wx.saturating_sub(sx),
        _ => cx - sx / 2,
    };
    let oy = match () {
        _ if cy < sy => 0,
        _ if cy > wy.saturating_sub(sy) => wy.saturating_sub(sy),
        _ => cy + 1 - sy,
    };
    Some((ox, oy))
}

#[cfg(test)]
mod tests {
    use super::tmux_window_offset;
    use crate::cmd::FormatVars;
    use crate::server::dispatch::options::{FormatCtx, ServerVars};

    #[tokio::test]
    async fn the_window_cell_size_is_the_one_its_panes_were_given() {
        let mut state = crate::server::test_state();
        let argv: Vec<String> = ["new-session", "-d", "-s", "a"].map(String::from).to_vec();
        crate::server::dispatch::run(&mut state, &crate::server::test_ctx(), &argv)
            .expect("new-session");
        let pane = *state.model.panes.keys().next().expect("a pane");
        let cell = |state: &crate::server::State| {
            let ctx = FormatCtx::new(&state.model, pane);
            let vars = ServerVars::new(state, pane, &ctx);
            let get = |n| vars.get(n).unwrap_or_default();
            format!("{}x{}", get("window_cell_width"), get("window_cell_height"))
        };
        assert_eq!(cell(&state), "16x32");
        state.model.panes[&pane]
            .handle
            .set_cell_px(Some((9, 19)))
            .expect("cell size");
        assert_eq!(cell(&state), "9x19");
    }

    #[test]
    fn the_offset_follows_the_cursor_as_tmux_does() {
        // Recorded on tmux 3.7b: a 120x60 window, an 80x24 client (23
        // rows under the status line), the cursor at 99,54.
        assert_eq!(
            tmux_window_offset((80, 23), (120, 60), None, Some((99, 54))),
            Some((40, 37))
        );
        assert_eq!(
            tmux_window_offset((80, 23), (80, 23), None, Some((5, 5))),
            None
        );
        assert_eq!(
            tmux_window_offset((80, 23), (100, 40), None, Some((0, 2))),
            Some((0, 0))
        );
        // In the middle: the cursor's column centred, its row at the bottom.
        assert_eq!(
            tmux_window_offset((80, 23), (200, 100), None, Some((100, 50))),
            Some((60, 28))
        );
        assert_eq!(
            tmux_window_offset((80, 23), (100, 40), None, None),
            Some((0, 0))
        );
        assert_eq!(
            tmux_window_offset((80, 23), (100, 40), Some((90, 5)), None),
            Some((20, 5))
        );
    }
}
