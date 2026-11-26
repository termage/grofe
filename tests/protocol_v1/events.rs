//! The event catalogue the v1 fixtures are blessed from: one sample per
//! `Event` variant, and the variants' second shapes.

use grofe::event::Event;
use grofe::types::{PaneId, SessionId, WindowId};

/// One sample per `Event` variant, in the order of [`index`].
pub fn event_samples() -> Vec<Event> {
    let (w, s, p) = (WindowId(1), SessionId(2), PaneId(3));
    vec![
        Event::PaneExited {
            pane: PaneId(4),
            uid: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
            status: Some(0),
        },
        Event::PaneDied {
            pane: p,
            uid: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
            window: w,
            status: Some(1),
        },
        Event::pane_output(p, b"hi\r\n"),
        Event::PaneFocus {
            pane: p,
            window: w,
            session: s,
            client: "client-1".into(),
        },
        Event::PaneResized { pane: p, window: w },
        Event::WindowFocused {
            window: w,
            session: s,
            pane: p,
        },
        Event::WindowCreated {
            window: w,
            session: s,
            pane: p,
        },
        Event::WindowResized {
            window: w,
            session: s,
        },
        Event::SessionCreated {
            session: s,
            name: "main".into(),
        },
        Event::SessionClosed {
            session: s,
            name: "main".into(),
        },
        Event::ClientSessionChanged {
            client: "client-1".into(),
            session: s,
        },
        Event::ClientResized {
            client: "client-1".into(),
            session: s,
            cols: 200,
            rows: 50,
        },
        Event::AlertBell {
            window: w,
            session: s,
            pane: p,
        },
        Event::AlertActivity {
            window: w,
            session: s,
            pane: p,
        },
        Event::AlertSilence {
            window: w,
            session: s,
            pane: p,
        },
        Event::OptionChanged {
            scope: "global".into(),
            name: "status".into(),
            value: Some("on".into()),
        },
        Event::AgentChanged {
            pane: p,
            uid: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
            kind: Some("claude".into()),
            old: Some("working".into()),
            new: Some("blocked".into()),
            msg: None,
        },
        Event::OpenUrl {
            url: "https://example.com/a".into(),
            pane: Some(p),
        },
        Event::PortsChanged {
            pane: p,
            ports: vec![3000, 5173],
        },
        Event::PaneEcho {
            pane: p,
            echo: false,
        },
        Event::Notify {
            pane: p,
            session: Some(s),
            kind: Some("claude".into()),
            state: "blocked".into(),
            msg: Some("Bash ls".into()),
        },
        Event::EventsDropped { count: 3 },
        Event::PaneCreated {
            pane: p,
            window: w,
            session: s,
        },
        Event::PaneClosed { pane: p },
    ]
}

/// The variant's position in [`event_samples`]. No wildcard: a new `Event`
/// variant does not compile here until it has a sample, and so a fixture.
pub fn index(event: &Event) -> usize {
    match event {
        Event::PaneExited { .. } => 0,
        Event::PaneDied { .. } => 1,
        Event::PaneOutput { .. } => 2,
        Event::PaneFocus { .. } => 3,
        Event::PaneResized { .. } => 4,
        Event::WindowFocused { .. } => 5,
        Event::WindowCreated { .. } => 6,
        Event::WindowResized { .. } => 7,
        Event::SessionCreated { .. } => 8,
        Event::SessionClosed { .. } => 9,
        Event::ClientSessionChanged { .. } => 10,
        Event::ClientResized { .. } => 11,
        Event::AlertBell { .. } => 12,
        Event::AlertActivity { .. } => 13,
        Event::AlertSilence { .. } => 14,
        Event::OptionChanged { .. } => 15,
        Event::AgentChanged { .. } => 16,
        Event::OpenUrl { .. } => 17,
        Event::PortsChanged { .. } => 18,
        Event::PaneEcho { .. } => 19,
        Event::Notify { .. } => 20,
        Event::EventsDropped { .. } => 21,
        Event::PaneCreated { .. } => 22,
        Event::PaneClosed { .. } => 23,
    }
}

/// Events whose second shape (a null) is fixed too.
pub fn event_extras() -> Vec<(&'static str, Event)> {
    vec![
        (
            "event-pane-exited-killed",
            Event::PaneExited {
                pane: PaneId(4),
                uid: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
                status: None,
            },
        ),
        (
            "event-agent-changed-msg",
            Event::AgentChanged {
                pane: PaneId(4),
                uid: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
                kind: None,
                old: None,
                new: Some("blocked".into()),
                msg: Some("needs you".into()),
            },
        ),
        (
            "event-open-url-no-pane",
            Event::OpenUrl {
                url: "http://localhost:8080/".into(),
                pane: None,
            },
        ),
        (
            "event-notify-bare",
            Event::Notify {
                pane: PaneId(4),
                session: None,
                kind: None,
                state: "done".into(),
                msg: None,
            },
        ),
        (
            "event-ports-changed-none",
            Event::PortsChanged {
                pane: PaneId(4),
                ports: vec![],
            },
        ),
        (
            "event-option-changed-unset",
            Event::OptionChanged {
                scope: "$2".into(),
                name: "@x".into(),
                value: None,
            },
        ),
    ]
}
