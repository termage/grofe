use super::*;

/// At most one frame per client per this long (spec §4.4: ~16 ms).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// How long a status-line message stays up (tmux's `display-time`), when
/// the session has no `display-time` option.
pub const DISPLAY_TIME: Duration = Duration::from_millis(750);
/// tmux 3.7b's default `message-format`.
pub(crate) const MESSAGE_FORMAT: &str =
    "#[#{?#{command_prompt},#{E:message-command-style},#{E:message-style}}]#{message}";

/// How soon after a pane changed its window's name is checked for
/// `automatic-rename` (tmux's `NAME_INTERVAL`).
pub const NAME_INTERVAL: Duration = Duration::from_millis(500);

/// How long a client must stay the latest active one before the window
/// takes its size (spec D6: ~500 ms).
pub const SIZE_DEBOUNCE: Duration = Duration::from_millis(500);

/// What an attach hello asks for, as the connection hands it to the actor.
#[derive(Debug)]
pub struct AttachRequest {
    pub ctx: ClientCtx,
    /// The client terminal, `(cols, rows)`.
    pub size: (u16, u16),
    pub term: String,
    pub caps: Caps,
    /// The attaching process, from the socket's peer credentials
    /// (`client_pid`).
    pub pid: Option<i32>,
    /// `attach -t`, raw.
    pub target: Option<String>,
    pub detach_others: bool,
    pub nest: bool,
    /// The client's terminal device, if it said (`client_tty`).
    pub tty: Option<String>,
    /// Its own `new-session` made the session: no `client-attached`.
    pub created: bool,
    /// `attach-session -c -E -f -r -x`.
    pub extras: crate::protocol::AttachExtras,
}

/// An accepted attach: the connection writes `output` (one frame at a
/// time) and, once the actor lets go of the sender, the reason from `exit`.
#[derive(Debug)]
pub struct Accepted {
    pub id: ClientId,
    pub output: mpsc::Receiver<Vec<u8>>,
    /// Dropped without a value when the server goes away mid-write.
    pub exit: oneshot::Receiver<String>,
    /// A value asks the client to suspend itself (`suspend-client`, TC.9):
    /// its connection sends a `suspend` frame after the output before it.
    pub suspend: mpsc::Receiver<()>,
}

/// A client's one-slot output: the actor offers each new screen, and the
/// diff goes out only if the slot is free.
pub struct Outbox {
    pub(super) tx: mpsc::Sender<Vec<u8>>,
    /// The screen the client shows once everything sent so far is written;
    /// `None` forces a full redraw.
    pub(super) prev: Option<Screen>,
    /// Set when a diff was dropped: nothing is composed for this client
    /// until its slot is free again.
    pub(super) blocked: bool,
    /// Bytes for ahead of the next frame ([`queue_raw`]); kept across a
    /// dropped frame.
    pub(super) raw: Vec<u8>,
    /// Bytes of dropped diffs, ever (`client_discarded`).
    pub(super) discarded: u64,
    /// The images the client's terminal shows once everything sent so far
    /// is written (G3); unlike `prev`, kept across a dropped frame.
    pub(super) placed: Vec<ScreenImage>,
}

/// Most raw bytes held for a client whose frames are being dropped.
// ponytail: past this, newer raw bytes are dropped until the client reads
// again; a full redraw re-sends what must survive (graphics, T3.8).
pub(super) const RAW_CAP: usize = 16 * 1024 * 1024;

impl Outbox {
    /// An outbox and the receiver its connection writes from.
    pub fn new() -> (Outbox, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel(1);
        let outbox = Outbox {
            tx,
            prev: None,
            blocked: false,
            raw: Vec::new(),
            discarded: 0,
            placed: Vec::new(),
        };
        (outbox, rx)
    }

    /// Whether offering a frame now is worth composing one.
    pub fn ready(&self) -> bool {
        !self.blocked || self.tx.capacity() > 0
    }

    /// Whether the next offer is a full redraw.
    pub fn needs_full_redraw(&self) -> bool {
        self.prev.is_none()
    }

    /// Holds `bytes` for ahead of the next frame; false if they would pass
    /// [`RAW_CAP`] and were dropped.
    pub fn queue_raw(&mut self, bytes: &[u8]) -> bool {
        let fits = self.raw.len() + bytes.len() <= RAW_CAP;
        if fits {
            self.raw.extend_from_slice(bytes);
        }
        fits
    }

    /// Sends the queued raw bytes, then the diff that turns the client's
    /// screen into `next`, as ONE frame (so a synchronized-output begin/end
    /// pair is never split, and raw bytes land before it). If the slot is
    /// still full the frame is dropped, the raw bytes stay queued, and the
    /// next offer is a full redraw. Returns false for a dropped frame: the
    /// client still needs one, and must be offered it again once
    /// [`Outbox::ready`].
    pub fn offer(&mut self, next: Screen, caps: &Caps) -> bool {
        let diff = render::diff_placed(self.prev.as_ref(), &next, caps, &self.placed);
        if diff.is_empty() && self.raw.is_empty() {
            return true;
        }
        let mut bytes = std::mem::take(&mut self.raw);
        let raw_len = bytes.len();
        bytes.extend_from_slice(&diff);
        match self.tx.try_send(bytes) {
            Ok(()) => {
                self.placed.clone_from(&next.images);
                self.prev = Some(next);
                self.blocked = false;
                true
            }
            Err(mpsc::error::TrySendError::Full(mut bytes)) => {
                self.discarded += (bytes.len() - raw_len) as u64;
                bytes.truncate(raw_len);
                self.raw = bytes;
                self.prev = None;
                self.blocked = true;
                false
            }
            // The connection is gone; its `ClientGone` removes the client.
            Err(mpsc::error::TrySendError::Closed(_)) => true,
        }
    }
}

/// One attached client, keyed by `ClientId` in `State::clients`.
pub struct ClientEntry {
    /// `client_name`, what `detach-client -t` takes: its tty when it
    /// attached in the tmux dialect, as tmux names clients, else
    /// `client-N`.
    pub name: String,
    /// `client_tty`: the client's terminal device, if it said.
    pub tty: Option<String>,
    /// `client_pid`: the client process.
    pub pid: Option<i32>,
    pub session: SessionId,
    /// The attaching CLI's context, with `client` set to this client.
    pub ctx: ClientCtx,
    /// The client terminal, `(cols, rows)`.
    pub size: (u16, u16),
    /// The latest Resize frame's size, applied on the next tick so a
    /// burst of resizes costs one reflow.
    pub(crate) pending_size: Option<(u16, u16)>,
    /// `TERM` of the client's terminal.
    pub term: String,
    pub caps: Caps,
    /// Last input (or the attach itself).
    pub activity: Instant,
    /// Wall-clock last input or attach (`client_activity`; unlike
    /// `activity`, a switch does not count, as in tmux).
    pub activity_time: SystemTime,
    /// When it attached (`client_created`).
    pub created: SystemTime,
    pub decoder: Decoder,
    pub keys: KeyState,
    /// Whether the outer terminal has focus, from its focus reports (the
    /// client asks for them); a client starts focused, as in tmux.
    pub focused: bool,
    /// The mouse report before the one being routed, as tmux's `m->lx`,
    /// `m->ly` and `m->lb` carry it: client cell and report code.
    pub mouse_prev: Option<(u16, u16, u32)>,
    pub(super) mouse_last: Option<(u16, u16, u32)>,
    /// A message shown on the status line's first row until it expires or
    /// a key is pressed (a failed binding's error, as tmux shows it), in
    /// the language [`status::draw`] reads, and when it expires (`None`
    /// with `display-time 0`: only a key takes it down, as in tmux), and
    /// its level, for `message-pill`.
    pub(super) message: Option<(String, Option<Instant>, render::Level)>,
    /// `display-message -N`: a key does not take the message down.
    pub(super) message_ignores_keys: bool,
    /// The status rows as last drawn, top first, for their click ranges,
    /// and the client row the first of them was drawn on.
    pub(super) status_rows: Vec<StatusRow>,
    pub(super) status_at: u16,
    /// The window cell drawn at the client's top-left (crop, D6), as last
    /// drawn.
    pub viewport: (u16, u16),
    /// Set by `refresh-client -L/-R/-U/-D` (TC.9): the window whose
    /// visible part the client moved by hand, and where it is; the view
    /// then stops following the cursor until `-c`, another window, or the
    /// window fitting the client, as tmux's `pan_window`.
    pub(crate) pan: Option<(WindowId, u16, u16)>,
    /// `suspend-client` stopped the client (TC.9): nothing is drawn for it
    /// until it resumes and sends its size, which redraws it in full.
    pub(super) suspended: bool,
    pub(super) suspend_tx: mpsc::Sender<()>,
    /// `read-only`, `ignore-size` and the like.
    pub flags: ClientFlags,
    /// The outer terminal's mouse modes as last written (R2): 0 off, 1 on,
    /// 2 on with all motion; `None` before the first write.
    pub(super) mouse_modes: Option<u8>,
    /// Input held while a binding's shell condition runs ([`hold_input`]),
    /// in order, and how many conditions hold it.
    pub(super) held: VecDeque<Held>,
    pub(super) held_bytes: usize,
    pub(super) holds: u32,
    /// The hold passed its cap: all input is dropped until it is released.
    pub(super) held_full: bool,
    /// Needs a new frame.
    pub(super) dirty: bool,
    /// When the toast last shown here expires, as [`deadline`] last saw
    /// it: the toast is gone by then, so [`tick`] cannot work it out.
    pub(super) toast_until: std::cell::Cell<Option<Instant>>,
    /// What a remote client said about itself (`set-client-info`).
    pub(crate) info: ClientInfo,
    pub(super) outbox: Outbox,
    pub(super) exit: Option<oneshot::Sender<String>>,
}

/// `set-client-info`'s values: `#{client_rtt}` and `#{client_machines}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    /// The smoothed round trip time, in milliseconds.
    pub rtt_ms: Option<u64>,
    /// The client's machine list, as it wants it shown.
    pub machines: Option<String>,
}

/// What `refresh-client -f`, `attach-session -f -r` and `switch-client -r`
/// set on a client (tmux's `CLIENT_READONLY` and its kin).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientFlags {
    /// The client's keys, pastes and mouse reach no pane, and only the
    /// commands tmux marks read-only run from its bindings.
    pub read_only: bool,
    /// The client does not size the windows of its session.
    pub ignore_size: bool,
    /// The client stays attached when its session is destroyed.
    pub no_detach_on_destroy: bool,
    /// tmux gives such a client its own active pane per window; grofe keeps
    /// the flag (and shows it) but not that.
    pub active_pane: bool,
}

impl ClientFlags {
    /// `server_client_set_flags`: `flags` is a comma list of names, each
    /// switched off by a `!`; a name tmux does not know is skipped.
    pub fn apply(&mut self, flags: &str) {
        for word in flags.split(',') {
            let (on, name) = match word.strip_prefix('!') {
                Some(name) => (false, name),
                None => (true, word),
            };
            match name {
                // tmux masks `read-only` out of the flags it clears: a
                // client cannot be made writable this way.
                "read-only" => self.read_only |= on,
                "ignore-size" => self.ignore_size = on,
                "no-detach-on-destroy" => self.no_detach_on_destroy = on,
                "active-pane" => {
                    if on && !self.active_pane {
                        crate::compat::note_miss("client flag active-pane: cosmetic, ignored");
                    }
                    self.active_pane = on;
                }
                _ => {}
            }
        }
    }

    /// The names `#{client_flags}` lists for these, in tmux's order.
    pub(super) fn names(self) -> Vec<&'static str> {
        [
            (self.ignore_size, "ignore-size"),
            (self.no_detach_on_destroy, "no-detach-on-destroy"),
            (self.read_only, "read-only"),
            (self.active_pane, "active-pane"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }
}

impl ClientEntry {
    /// `suspend-client` stopped the client and it has not resumed.
    pub(crate) fn is_suspended(&self) -> bool {
        self.suspended
    }
}

/// Which client a session's windows are sized for (spec D6), and the
/// client about to take over once its debounce runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeOwner {
    pub client: ClientId,
    pub(super) pending: Option<(ClientId, Instant)>,
}

impl SizeOwner {
    pub fn new(client: ClientId) -> SizeOwner {
        SizeOwner {
            client,
            pending: None,
        }
    }

    /// `client` sent input at `now`. The owner typing again cancels a
    /// pending switch; another client becomes the owner [`SIZE_DEBOUNCE`]
    /// after its first input unless the owner types meanwhile.
    pub fn activity(&mut self, client: ClientId, now: Instant) {
        if client == self.client {
            self.pending = None;
        } else if self.pending.is_none_or(|(c, _)| c != client) {
            self.pending = Some((client, now + SIZE_DEBOUNCE));
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, at)| at)
    }

    /// Completes a due switch; whether the owner changed.
    pub fn settle(&mut self, now: Instant) -> bool {
        match self.pending {
            Some((client, at)) if now >= at => {
                self.client = client;
                self.pending = None;
                true
            }
            _ => false,
        }
    }
}

/// Client state that is not per client.
pub struct Registry {
    pub(super) next_id: u32,
    /// Bindings by table, starting with the default key table.
    pub key_tables: KeyTables,
    /// See every input before the key tables, in registration order.
    pub input_hooks: Vec<Arc<dyn InputHook + Send + Sync>>,
    /// Drawn over every client's frame, in reverse registration order.
    pub overlays: Vec<Arc<dyn Overlay + Send + Sync>>,
    pub(super) owners: BTreeMap<SessionId, SizeOwner>,
    pub(super) render_at: Option<Instant>,
    pub(super) last_render: Option<Instant>,
    /// When the status line is next redrawn for its clock.
    pub(super) status_at: Option<Instant>,
    /// When window names are next checked (`automatic-rename`).
    pub(super) names_at: Option<Instant>,
    /// The status-line messages `show-messages` prints (TC.9), oldest
    /// first, with when each was shown.
    pub(super) messages: VecDeque<(SystemTime, String)>,
    /// The options a `set-option` has already warned about: stored, but
    /// with no effect in grofe.
    pub(super) inert_warned: BTreeSet<String>,
    /// `wait-for` channels (TC.9).
    pub(crate) waits: BTreeMap<String, crate::server::dispatch::client::WaitChannel>,
    /// The clients now locked, their `lock-command` running in a popup
    /// (`dispatch::lock`).
    pub(crate) locked: BTreeSet<ClientId>,
    /// Per session, the client activity its `lock-after-time` last fired
    /// for: tmux's lock timer is one-shot, so it waits for newer activity.
    pub(crate) lock_fired: BTreeMap<SessionId, Instant>,
    /// `server-access`: the users allowed besides the owner, and whether
    /// each may only read (uid, read-only). Root and the server's own user
    /// are always in it, as tmux's `server_acl_init`.
    pub(super) acl: BTreeMap<u32, bool>,
    /// When each pane was last scanned for an agent program.
    // ponytail: never pruned except by `scan_agents` dropping gone panes,
    // so it holds at most one instant per pane that has produced output.
    pub(super) agent_scans: BTreeMap<PaneId, Instant>,
    /// `agents.scan-interval`; the config sets it with `set_scan_interval`.
    pub(super) scan_interval: Duration,
}

impl Registry {
    #[cfg(test)]
    pub(crate) fn scanned(&self, pane: PaneId) -> bool {
        self.agent_scans.contains_key(&pane)
    }

    /// The next scan of `pane` is not throttled.
    pub(crate) fn forget_scan(&mut self, pane: PaneId) {
        self.agent_scans.remove(&pane);
    }

    pub(crate) fn acl_contains(&self, uid: u32) -> bool {
        self.acl.contains_key(&uid)
    }

    pub(crate) fn acl_entries(&self) -> impl Iterator<Item = (u32, bool)> + '_ {
        self.acl.iter().map(|(uid, ro)| (*uid, *ro))
    }

    pub(crate) fn acl_allow(&mut self, uid: u32) {
        self.acl.entry(uid).or_insert(false);
    }

    pub(crate) fn acl_deny(&mut self, uid: u32) {
        self.acl.remove(&uid);
    }

    pub(crate) fn acl_set_read_only(&mut self, uid: u32, read_only: bool) {
        if let Some(entry) = self.acl.get_mut(&uid) {
            *entry = read_only;
        }
    }
}

impl Default for Registry {
    fn default() -> Registry {
        Registry {
            next_id: 1,
            key_tables: default_bindings(),
            input_hooks: Vec::new(),
            overlays: Vec::new(),
            owners: BTreeMap::new(),
            render_at: None,
            last_render: None,
            status_at: None,
            names_at: None,
            messages: VecDeque::new(),
            inert_warned: BTreeSet::new(),
            waits: BTreeMap::new(),
            locked: BTreeSet::new(),
            lock_fired: BTreeMap::new(),
            acl: [0, nix::unistd::getuid().as_raw()]
                .into_iter()
                .map(|uid| (uid, false))
                .collect(),
            agent_scans: BTreeMap::new(),
            scan_interval: Duration::from_secs(1),
        }
    }
}
