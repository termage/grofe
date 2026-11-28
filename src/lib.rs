//! Module declarations.

pub mod agent_cli;
pub mod agent_setup;
pub mod client;
pub mod cmd;
pub mod compat;
pub mod config;
pub mod control_mode;
pub mod event;
pub mod import_tmux;
pub mod layouts;
pub mod lua;
pub mod open;
pub mod pack;
pub mod paths;
pub mod protocol;
pub mod remote;
pub mod server;
pub mod term;
#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_exec;
pub mod types;
pub mod util;

/// The calling thread's CPU time: unlike wall time, a loaded host does not
/// stretch it, nor does waiting on a disk. Zero if it cannot be read.
pub(crate) fn thread_cpu_time() -> std::time::Duration {
    use nix::libc;
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec to write to.
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0;
    debug_assert!(ok, "clock_gettime(CLOCK_THREAD_CPUTIME_ID)");
    if !ok {
        // ponytail: no thread clock (never seen on Linux or macOS) means no
        // Lua budget; its wall limit still ends a call.
        return std::time::Duration::ZERO;
    }
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}
