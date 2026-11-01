//! destroy-unattached once the last client is gone: end the session now, or
//! wait while the process of the client the server lost is still running.
//!
//! The per client writer drops a connection whose client stopped reading for
//! 5 s (a frozen PC, a hung terminal, see client_frame_queue.rs). That client
//! is alive and reconnects once it reads again, so its drop is not a detach:
//! the session waits for it and ends only when its process has exited. A
//! client whose pid or creation time is unknown is treated as gone, which is
//! the old immediate teardown.

/// What the server does about destroy-unattached right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unattached {
    /// A client is attached or the option is off.
    Keep,
    /// The last client's process still runs: it may reconnect.
    Wait,
    /// End the session.
    DestroyNow,
}

/// A client process: its pid and the creation time it had when it attached.
/// The creation time tells a recycled pid from the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientIdentity {
    pub pid: u32,
    pub created: u64,
}

/// What a pid looks like now: whether it runs, and its creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessNow {
    pub alive: bool,
    pub created: Option<u64>,
}

/// `destroy_unattached` is the option's value; `probe` reads a pid now.
pub(crate) fn decide<F>(attached: usize, destroy_unattached: bool, last: Option<ClientIdentity>, probe: F) -> Unattached
where
    F: Fn(u32) -> ProcessNow,
{
    if !destroy_unattached || attached > 0 {
        return Unattached::Keep;
    }
    let Some(last) = last else { return Unattached::DestroyNow };
    if last.pid == 0 {
        return Unattached::DestroyNow;
    }
    let now = probe(last.pid);
    if now.alive && now.created == Some(last.created) {
        Unattached::Wait
    } else {
        Unattached::DestroyNow
    }
}

/// The real probe: liveness plus the creation time of the pid.
pub(crate) fn probe_process(pid: u32) -> ProcessNow {
    ProcessNow {
        alive: crate::platform::process_is_alive(pid),
        created: crate::platform::process_kill::process_creation_time(pid),
    }
}

/// The identity of a client attaching with `pid`, None when it cannot be read.
/// The server's own pid is no client process to wait for: it never exits
/// while the server runs.
pub(crate) fn identity_at_attach(pid: Option<u32>) -> Option<ClientIdentity> {
    let pid = pid.filter(|p| *p != 0 && *p != std::process::id())?;
    let created = crate::platform::process_kill::process_creation_time(pid)?;
    Some(ClientIdentity { pid, created })
}

#[cfg(test)]
#[path = "../../tests-rs/test_destroy_unattached_stalled_client.rs"]
mod tests_destroy_unattached_stalled_client;
