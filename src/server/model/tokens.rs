//! tmux's pane tokens (`{top}`, `{left-of}`, and the plain words `top`,
//! `Bottom-Left`) and the neighbour of a pane, resolved against a window.

use super::{HOLE, Model, Window};
use crate::cmd::{PanePart, PaneRef, PaneToken};
use crate::protocol::ClientCtx;
use crate::server::borders::PaneStatus;
use crate::server::layout;
use crate::server::mouse::border_pane;
use crate::types::{Edge, PaneId, Rect};

impl Model {
    /// A pane part that is `!` or one of tmux's pane tokens, in the window
    /// the target names.
    pub(super) fn last_or_token_pane(
        &self,
        r: &PaneRef,
        ctx: &ClientCtx,
    ) -> Result<PaneId, String> {
        let w = self.window_ref(&r.session, &r.window, ctx)?;
        let window = &self.windows[&w];
        match r.pane {
            PanePart::Token(token) => window
                .token_pane(token)
                .ok_or_else(|| format!("can't find pane: {}", token.name())),
            _ => window
                .last_pane()
                .ok_or_else(|| "can't find pane: !".to_string()),
        }
    }
}

impl Window {
    /// The pane `select-pane -L/-R/-U/-D` moves to from `pane`, found in the
    /// unzoomed layout as tmux's `window_pane_find_*` do: the most recently
    /// active one across the edge, wrapping past the window's edge.
    /// ponytail: floating panes are never candidates.
    pub(crate) fn adjacent_pane(&self, pane: PaneId, edge: Edge) -> Option<PaneId> {
        layout::find_adjacent(
            &self.tiled_rects(),
            self.area(),
            self.pane_status,
            pane,
            edge,
            |p| self.active_points.get(p),
        )
    }

    /// The pane tmux's `{top}`, `{bottom-left}` and the like name: the
    /// first pane covering that cell of the window, a pane's right and
    /// bottom border cells included (window.c `window_get_active_at`, with
    /// `window_find_string`'s cells). `{up-of}` and the other `-of` tokens
    /// are the active pane's neighbour.
    pub(crate) fn token_pane(&self, token: PaneToken) -> Option<PaneId> {
        let edge = match token {
            PaneToken::UpOf => Some(Edge::Up),
            PaneToken::DownOf => Some(Edge::Down),
            PaneToken::LeftOf => Some(Edge::Left),
            PaneToken::RightOf => Some(Edge::Right),
            _ => None,
        };
        if let Some(edge) = edge {
            return self.adjacent_pane(self.active, edge);
        }
        let (cols, rows) = (u32::from(self.cols), u32::from(self.rows));
        let (last_x, last_y) = (cols.saturating_sub(1), rows.saturating_sub(1));
        let (x, y) = match token {
            PaneToken::Top => (cols / 2, 0),
            PaneToken::Bottom => (cols / 2, last_y),
            PaneToken::Left => (0, rows / 2),
            PaneToken::Right => (last_x, rows / 2),
            PaneToken::TopLeft => (0, 0),
            PaneToken::TopRight => (last_x, 0),
            PaneToken::BottomLeft => (0, last_y),
            _ => (last_x, last_y),
        };
        // A framed window has gaps between its frames: the nearest frame's
        // pane owns a gap cell, as it does for the mouse.
        if self.framed() {
            return crate::server::mouse::frame_owner(self, (x, y));
        }
        // The pane-status line above a pane is part of it.
        let above = u16::from(self.pane_status == PaneStatus::Top);
        let panes: Vec<_> = self
            .pane_rects()
            .into_iter()
            .filter(|(id, _)| *id != HOLE)
            .map(|(id, r)| {
                let up = above.min(r.y);
                let r = Rect {
                    y: r.y - up,
                    rows: r.rows + up,
                    ..r
                };
                (id, r)
            })
            .collect();
        border_pane(&panes, (x, y))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime};

    use super::*;
    use crate::cmd::Resolve;
    use crate::cmd::target::{parse_target, with_tmux_targets};
    use crate::protocol::Dialect;
    use crate::server::layout::LayoutTree;
    use crate::server::model::{Model, Session};
    use crate::types::{Dir, SessionId, SplitSize, WindowId};

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        cols: 80,
        rows: 24,
    };

    /// An 80x24 window of `layout` with `active` selected.
    fn window(layout: LayoutTree, active: PaneId) -> Window {
        Window {
            id: WindowId(0),
            session: SessionId(0),
            index: 0,
            name: "w".to_string(),
            layout,
            active,
            zoomed: false,
            cols: 80,
            rows: 24,
            bell_flag: false,
            activity_flag: false,
            silence_flag: false,
            activity: SystemTime::now(),
            last_panes: Vec::new(),
            active_points: Default::default(),
            floating: Vec::new(),
            pane_status: Default::default(),
            frames: false,
            pane_gap: 0,
            drop_preview: None,
            links: Vec::new(),
        }
    }

    /// A model with session `s` and one window holding `layout`.
    fn model_with(layout: LayoutTree, active: PaneId) -> Model {
        let mut model = Model::new(true);
        let id = model.alloc_session_id();
        let wid = model.alloc_window_id();
        model.insert_session(Session {
            id,
            name: "s".to_string(),
            windows: Vec::new(),
            active: wid,
            env: Vec::new(),
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            last_windows: Vec::new(),
            activity: Instant::now(),
            attached: 0,
            created: SystemTime::now(),
            last_attached: None,
        });
        model.insert_window(Window {
            id: wid,
            session: id,
            ..window(layout, active)
        });
        model
    }

    fn ctx(dialect: Dialect) -> ClientCtx {
        ClientCtx {
            cwd: std::env::temp_dir(),
            pane: None,
            env: Vec::new(),
            client: None,
            dialect,
            request: Default::default(),
        }
    }

    /// `a` split into `a | b`.
    fn pair(a: PaneId, b: PaneId) -> LayoutTree {
        let mut layout = LayoutTree::new(a);
        layout
            .split(AREA, a, Dir::Horizontal, b, SplitSize::Half)
            .unwrap();
        layout
    }

    /// tmux's pane tokens on a 2x2 grid: A top left, B top right, C bottom
    /// left, D bottom right (80x24; recorded on tmux 3.7b: `{top}` is the
    /// pane at the top middle cell, the `-of` ones are the neighbours of
    /// the active pane, wrapping).
    #[test]
    fn pane_tokens_pick_by_edge_and_neighbour() {
        let (a, b, c, d) = (PaneId(1), PaneId(2), PaneId(3), PaneId(4));
        let mut layout = LayoutTree::new(a);
        layout
            .split(AREA, a, Dir::Horizontal, b, SplitSize::Half)
            .unwrap();
        layout
            .split(AREA, a, Dir::Vertical, c, SplitSize::Half)
            .unwrap();
        layout
            .split(AREA, b, Dir::Vertical, d, SplitSize::Half)
            .unwrap();
        let mut w = window(layout, a);
        use PaneToken::*;
        let picks = |w: &Window, tokens: [PaneToken; 4]| tokens.map(|t| w.token_pane(t));
        let edges = [
            Top,
            Bottom,
            Left,
            Right,
            TopLeft,
            TopRight,
            BottomLeft,
            BottomRight,
        ];
        assert_eq!(
            edges.map(|t| w.token_pane(t)),
            [a, c, a, b, a, b, c, d].map(Some)
        );
        let of = [UpOf, DownOf, LeftOf, RightOf];
        assert_eq!(picks(&w, of), [c, c, b, b].map(Some));
        w.active = d;
        assert_eq!(picks(&w, of), [b, b, c, c].map(Some));
        w.active = b;
        assert_eq!(picks(&w, of), [d, d, a, a].map(Some));
    }

    /// The tokens resolve as a bare pane target and after `window.`, in
    /// either dialect, and `{mouse}` is tmux's mouse target.
    #[test]
    fn pane_tokens_resolve_as_targets() {
        let (a, b) = (PaneId(1), PaneId(2));
        let model = model_with(pair(a, b), a);
        let ctx = ctx(Dialect::Grofe);
        let pane = |text: &str| model.resolve_pane(&parse_target(text).unwrap(), &ctx);
        assert_eq!(pane("{right-of}"), Ok(b));
        assert_eq!(pane("{left-of}"), Ok(b));
        assert_eq!(pane("{bottom-right}"), Ok(b));
        assert_eq!(pane(":0.{top-left}"), Ok(a));
        assert_eq!(pane("s:0.{right}"), Ok(b));
        assert_eq!(pane("{up-of}"), Err("can't find pane: {up-of}".to_string()));
        assert_eq!(pane("{mouse}"), Err("no mouse target".to_string()));
        let tmux = |text: &str| with_tmux_targets(|| parse_target(text).unwrap());
        assert_eq!(model.resolve_pane(&tmux("{left}"), &ctx), Ok(a));
        assert_eq!(
            model.resolve_pane(&tmux("{mouse}"), &ctx),
            Err("no mouse target".to_string())
        );
        assert_eq!(model.resolve_pane(&tmux(".{right}"), &ctx), Ok(b));
    }

    /// tmux also takes the plain words (`select-pane -t top`, `-t s:0.Right`)
    /// in any case; a grofe client's bare word may be a session's name, so
    /// there `top` stays one.
    #[test]
    fn pane_token_words_resolve_for_tmux_only() {
        let (a, b) = (PaneId(1), PaneId(2));
        let model = model_with(pair(a, b), a);
        let tmux_ctx = ctx(Dialect::Tmux);
        let pane = |text: &str| {
            model.resolve_pane(
                &with_tmux_targets(|| parse_target(text).unwrap()),
                &tmux_ctx,
            )
        };
        assert_eq!(pane("top"), Ok(a));
        assert_eq!(pane("Right"), Ok(b));
        assert_eq!(pane("BOTTOM-LEFT"), Ok(a));
        assert_eq!(pane("s:0.Right"), Ok(b));
        assert_eq!(pane("0.top-right"), Ok(b));
        assert_eq!(pane("up-of"), Err("can't find pane: up-of".to_string()));
        let grofe = model.resolve_pane(&parse_target("top").unwrap(), &ctx(Dialect::Grofe));
        assert_eq!(grofe, Err("can't find pane: top".to_string()));
    }
}
