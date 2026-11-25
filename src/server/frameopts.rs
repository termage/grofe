//! The frame options an overlay framed like the window's panes reads.

use crate::cmd::FormatVars;
use crate::server::State;
use crate::server::borders::PaneLines;
use crate::server::dispatch::options::{self, Owner};
use crate::server::model::Model;
use crate::server::render::{self, PaneLooks};
use crate::server::status::{self, Style};
use crate::types::ClientId;

/// The frame options of a client's current window, for an overlay that
/// frames itself as the window's panes are framed: their lines (as
/// [`render::apply_frames`] settles them) and title switch, and any option or
/// style looked up from the window's active pane.
pub struct FrameOptions<'a> {
    pub lines: PaneLines,
    pub title: bool,
    model: &'a Model,
    chain: Vec<Owner>,
    vars: &'a dyn FormatVars,
}

impl FrameOptions<'_> {
    pub fn opt(&self, name: &str) -> &str {
        self.model.options.lookup(name, &self.chain).unwrap_or("")
    }

    pub fn style(&self, name: &str) -> Style {
        status::base_style(self.opt(name), self.vars)
    }
}

/// Calls `f` with the [`FrameOptions`] of `client`'s current window;
/// `None` without a client or window.
pub fn with_frame_options<R>(
    state: &State,
    client: ClientId,
    f: impl FnOnce(&FrameOptions) -> R,
) -> Option<R> {
    let model = &state.model;
    let c = state.clients.get(&client)?;
    let w = model
        .sessions
        .get(&c.session)
        .and_then(|s| model.windows.get(&s.active))?;
    let chain = options::chain(model, w.active);
    let mut looks = PaneLooks {
        lines: PaneLines::parse(
            model
                .options
                .lookup("pane-border-lines", &chain)
                .unwrap_or(""),
        ),
        ..Default::default()
    };
    render::apply_frames(model, w, &mut looks);
    let inner = options::FormatCtx::new(model, w.active);
    let vars = options::ServerVars::fresh(state, w.active, &inner);
    Some(f(&FrameOptions {
        lines: looks.lines,
        title: looks.frame_title,
        model,
        chain,
        vars: &vars,
    }))
}
