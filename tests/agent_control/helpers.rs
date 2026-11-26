//! A fake agent that prints a prompt, a spinner or an approval line when it
//! reads a cue, a detection file that matches those lines, and small readers.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use crate::common::TestServer;

pub(crate) const WAIT: Duration = Duration::from_secs(10);

/// Cues, one per line on stdin. Every cue clears the screen first, so only
/// what the cue prints is on it.
const FAKE_AGENT: &str = r#"#!/bin/sh
clear() { printf '\033[2J\033[H'; }
clear
echo "READY>"
while IFS= read -r cue; do
  case "$cue" in
    work) clear; echo "WORKING: thinking"; sleep 1; clear; echo "READY>" ;;
    busy) clear; echo "esc to interrupt" ;;
    spin) clear; echo "WORKING: thinking" ;;
    ask) clear; echo "APPROVE? [y/n]" ;;
    both) clear; echo "WORKING: thinking"; echo "APPROVE? [y/n]" ;;
    ready) clear; echo "READY>" ;;
    buried) clear; echo "WORKING: thinking"; i=0; while [ $i -lt 6 ]; do echo "filler $i"; i=$((i+1)); done ;;
    lines) clear; i=1; while [ $i -lt 50 ]; do echo "line $i"; i=$((i+1)); done; printf 'line 50' ;;
    alt) printf '\033[?1049h\033[2J\033[H'; printf 'alt-screen line' ;;
    quit) exit 0 ;;
  esac
done
"#;

/// The detection file that matches the fake agent's lines.
pub(crate) const DETECT: &str =
    "working = \"WORKING:\"\nblocked = \"APPROVE\\\\?\"\nidle = \"READY>\"\n";

pub(crate) struct Fx {
    pub srv: TestServer,
    pub script: PathBuf,
}

/// A server whose config knows the kind `fake` (the script, matched by its
/// name) and holds `detection` as `agents/fake.toml` when given; session
/// `main` runs `cat`.
pub(crate) fn start(detection: Option<&str>) -> Fx {
    let srv = TestServer::builder().unstarted();
    let script = write_script(&srv);
    write_config(&srv, &script, "");
    if let Some(text) = detection {
        write_detection(&srv, "fake", text);
    }
    srv.cmd(&["start-server"]).expect("start-server");
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "24",
        "cat",
    ])
    .expect("new-session");
    Fx { srv, script }
}

pub(crate) fn write_script(srv: &TestServer) -> PathBuf {
    let script = srv.home.join("fake-agent");
    fs::write(&script, FAKE_AGENT).expect("write script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

/// `conf.d/50-agents.toml`: the kind `fake`, plus `extra` lines for it.
pub(crate) fn write_config(srv: &TestServer, script: &std::path::Path, extra: &str) {
    let conf = srv.config_home.join("grofe").join("conf.d");
    fs::create_dir_all(&conf).expect("conf.d");
    fs::write(
        conf.join("50-agents.toml"),
        format!(
            "[agents.kinds.fake]\ncommand = \"{0}\"\nmatch = [\"fake-agent\"]\n{extra}\n",
            script.display()
        ),
    )
    .expect("write config");
}

/// `<config dir>/agents/<kind>.toml`; returns its path.
pub(crate) fn write_detection(srv: &TestServer, kind: &str, text: &str) -> PathBuf {
    let dir = srv.config_home.join("grofe").join("agents");
    fs::create_dir_all(&dir).expect("agents dir");
    let path = dir.join(format!("{kind}.toml"));
    fs::write(&path, text).expect("write detection file");
    path
}

impl Fx {
    /// A new detached window running the fake agent as kind `fake`.
    pub(crate) fn agent(&self) -> String {
        self.agent_of("fake")
    }

    /// An agent pane whose program takes raw input (with bracketed paste on
    /// when asked) and writes every byte it gets to the returned file.
    pub(crate) fn raw_agent(&self, bracketed: bool) -> (String, PathBuf) {
        let dir = self.srv.home.join("raw");
        fs::create_dir_all(&dir).expect("raw dir");
        let (script, out) = (dir.join("fake-agent"), dir.join("bytes"));
        let paste = if bracketed {
            "printf '\\033[?2004h'\n"
        } else {
            ""
        };
        let text = format!(
            "#!/bin/sh\n{paste}stty raw -echo\necho 'READY>'\ncat > '{}'\n",
            out.display()
        );
        fs::write(&script, text).expect("write raw script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        let id = self
            .srv
            .cmd(&[
                "agent",
                "new",
                "fake",
                "-P",
                "-d",
                "--",
                script.to_str().expect("utf8"),
            ])
            .unwrap_or_else(|e| panic!("agent new: {e}; log:\n{}", self.srv.log()))
            .trim()
            .to_string();
        self.srv
            .wait_for(|| self.srv.capture(&id).contains("READY>"), WAIT)
            .unwrap_or_else(|e| panic!("{e}: raw agent never showed its prompt"));
        (id, out)
    }

    /// What the raw agent got, once `want` bytes of it are there.
    pub(crate) fn raw_bytes(&self, out: &std::path::Path, want: usize) -> Vec<u8> {
        self.srv
            .wait_for(|| fs::read(out).is_ok_and(|b| b.len() >= want), WAIT)
            .unwrap_or_else(|e| panic!("{e}: got {:?}", fs::read(out)));
        fs::read(out).expect("read bytes")
    }

    pub(crate) fn agent_of(&self, kind: &str) -> String {
        let id = self
            .srv
            .cmd(&[
                "agent",
                "new",
                kind,
                "-P",
                "-d",
                "--",
                self.script.to_str().expect("utf8"),
            ])
            .unwrap_or_else(|e| panic!("agent new: {e}; log:\n{}", self.srv.log()))
            .trim()
            .to_string();
        self.srv
            .wait_for(|| self.srv.capture(&id).contains("READY>"), WAIT)
            .unwrap_or_else(|e| panic!("{e}: fake agent never showed its prompt"));
        id
    }

    /// Types a cue and Enter into the pane, as a user would.
    pub(crate) fn cue(&self, pane: &str, cue: &str) {
        self.srv
            .cmd(&["send-keys", "-t", pane, "-l", cue])
            .expect("send-keys -l");
        self.srv
            .cmd(&["send-keys", "-t", pane, "Enter"])
            .expect("send-keys Enter");
    }

    /// The pane's `agent_state` ("" when it has no record).
    pub(crate) fn state(&self, pane: &str) -> String {
        self.srv
            .cmd(&["display-message", "-p", "-t", pane, "#{agent_state}"])
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    pub(crate) fn wait_state(&self, pane: &str, want: &str) {
        self.srv
            .wait_for(|| self.state(pane) == want, WAIT)
            .unwrap_or_else(|e| {
                panic!(
                    "{e}: {pane} is {:?}, want {want:?}; screen:\n{}",
                    self.state(pane),
                    self.srv.capture(pane)
                )
            });
    }

    pub(crate) fn explain(&self, pane: &str) -> Result<String, String> {
        self.srv.cmd(&["agent", "explain", pane])
    }
}

/// Runs a Lua snippet.
pub(crate) fn lua(srv: &TestServer, code: &str) -> String {
    srv.cmd(&["lua", code])
        .unwrap_or_else(|e| panic!("lua {code:?}: {e}\nlog:\n{}", srv.log()))
        .trim()
        .to_string()
}

/// Global user option `@r` ("" when unset).
pub(crate) fn opt_r(srv: &TestServer) -> String {
    srv.cmd(&["display-message", "-p", "#{@r}"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
