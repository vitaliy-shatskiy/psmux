//! destroy-unattached and a client the server dropped while its process runs
//! (a write stall): the session waits for that process instead of ending.

use super::*;

const CLIENT: ClientIdentity = ClientIdentity { pid: 4242, created: 133_000_000_000_000_000 };

fn running(_pid: u32) -> ProcessNow { ProcessNow { alive: true, created: Some(CLIENT.created) } }
fn exited(_pid: u32) -> ProcessNow { ProcessNow { alive: false, created: None } }
/// The pid now belongs to a process started after the client attached.
fn recycled(_pid: u32) -> ProcessNow { ProcessNow { alive: true, created: Some(CLIENT.created + 1) } }
/// The pid runs but its creation time cannot be read.
fn unreadable(_pid: u32) -> ProcessNow { ProcessNow { alive: true, created: None } }
fn must_not_probe(_pid: u32) -> ProcessNow { panic!("probed a pid it must not look at") }

#[test]
fn dropped_last_client_still_running_waits() {
    assert_eq!(decide(0, true, Some(CLIENT), running), Unattached::Wait,
        "BUG: a stalled but live client ended the session");
}

#[test]
fn last_client_exited_destroys_now() {
    assert_eq!(decide(0, true, Some(CLIENT), exited), Unattached::DestroyNow);
}

#[test]
fn recycled_pid_does_not_keep_the_session() {
    assert_eq!(decide(0, true, Some(CLIENT), recycled), Unattached::DestroyNow,
        "BUG: a recycled pid kept a dead client's session alive");
}

#[test]
fn unreadable_creation_time_destroys_now() {
    assert_eq!(decide(0, true, Some(CLIENT), unreadable), Unattached::DestroyNow);
}

#[test]
fn no_identity_destroys_now() {
    assert_eq!(decide(0, true, None, must_not_probe), Unattached::DestroyNow);
}

#[test]
fn pid_zero_destroys_now() {
    assert_eq!(decide(0, true, Some(ClientIdentity { pid: 0, created: CLIENT.created }), must_not_probe), Unattached::DestroyNow);
}

#[test]
fn a_client_attached_keeps_and_ends_the_wait() {
    assert_eq!(decide(1, true, Some(CLIENT), must_not_probe), Unattached::Keep);
}

#[test]
fn option_off_keeps() {
    assert_eq!(decide(0, false, Some(CLIENT), must_not_probe), Unattached::Keep);
}

#[test]
fn attach_identity_needs_a_pid() {
    assert_eq!(identity_at_attach(None), None);
    assert_eq!(identity_at_attach(Some(0)), None);
}

#[test]
fn the_servers_own_pid_is_not_a_client_to_wait_for() {
    assert_eq!(identity_at_attach(Some(std::process::id())), None);
}

#[cfg(windows)]
#[test]
fn the_real_probe_sees_a_running_process_with_its_creation_time() {
    let pid = std::process::id();
    let created = crate::platform::process_kill::process_creation_time(pid).expect("own creation time");
    assert_eq!(decide(0, true, Some(ClientIdentity { pid, created }), probe_process), Unattached::Wait);
    assert_eq!(decide(0, true, Some(ClientIdentity { pid, created: created + 1 }), probe_process), Unattached::DestroyNow,
        "BUG: the real probe let a different creation time keep the session");
}
