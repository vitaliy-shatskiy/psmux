//! Under a Russian or Ukrainian layout Ctrl + a letter key never
//! reached the program in the pane (Ctrl+V, Ctrl+O in Claude Code), and a
//! Ctrl prefix did not arm. crossterm names the key by the letter the layout
//! types, so Ctrl+V arrives as `C-м`: no binding matched it, the prefix check
//! compared it with `C-b`, and the server's `C-` arm took only an ASCII
//! letter and dropped it.
//!
//! The fix follows the #616 confirm prompt (`confirm::is_yes_with`): the key
//! keeps its name end to end, so a binding on `C-ы` works as before, and only
//! when nothing is bound to that name is it looked up as the Latin key of the
//! same physical key (`C-s`), found through the layout lookup passed in. The
//! layouts here are fakes, so no test reads the machine's live layout.
//!
//! Known gaps, left as they are:
//! - Ctrl+[ / Ctrl+] / Ctrl+\ on Russian: those keys type `х`, `ъ` and `\`,
//!   and only the letter keys (VK_A to VK_Z) are taken.
//! - Alt + a non Latin letter stays `M-ф`.
//! - A prefix table key on a plain Cyrillic letter (prefix then `с`) is not
//!   looked up as `c`: only Ctrl + a letter is.
//! - A layout whose letters are Latin script (Turkish `ı`, Polish `ł`) keeps
//!   its behaviour.
//! - The client's own prompt editors and paste detection see the key as
//!   typed: Ctrl+м is not held back as a possible paste the way Ctrl+V is.
//! - `send-keys C-ы` from a script is not a typed key and is not mapped.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::client::{find_binding, latin_arms_prefix};
use crate::input::send_key_to_active_with;
use crate::types::{AppState, Mode, Node, Pane, Window};

const CTRL: KeyModifiers = KeyModifiers::CONTROL;

// A Russian layout as Windows reports it (VkKeyScanExW's low byte): the S key
// (VK_S, 0x53) types `ы`, B types `и`, V types `м`, C types `с`, and the key
// right of P (VK_OEM_4) types `х`.
fn russian(c: char) -> Option<u16> {
    match c {
        'ы' | 'Ы' => Some(0x53),
        'и' | 'И' => Some(0x42),
        'м' | 'М' => Some(0x56),
        'с' | 'С' => Some(0x43),
        'х' => Some(0xDB),
        _ => None,
    }
}

// Turkish Q: the I key types the dotless `ı`, and `ş` sits on VK_OEM_1.
fn turkish_q(c: char) -> Option<u16> {
    match c {
        'ı' => Some(0x49),
        'ş' => Some(0xBA),
        c if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase() as u16),
        _ => None,
    }
}

// An English layout: every letter on its own key.
fn latin(c: char) -> Option<u16> {
    c.is_ascii_alphabetic().then(|| c.to_ascii_uppercase() as u16)
}

fn ctrl(c: char) -> (KeyCode, KeyModifiers) {
    (KeyCode::Char(c), CTRL)
}

// ── the Latin key of a Ctrl + letter ─────────────────────────────────────

#[test]
fn a_cyrillic_letter_stands_for_the_latin_key_it_is_typed_on() {
    assert_eq!(latin_ctrl_key_with(ctrl('ы'), russian), Some(ctrl('s')));
    assert_eq!(latin_ctrl_key_with(ctrl('м'), russian), Some(ctrl('v')));
    // Shift (or Caps Lock) keeps its case and its modifier, as an English
    // layout gives Char('S') with Ctrl+Shift+S.
    let shift = CTRL | KeyModifiers::SHIFT;
    assert_eq!(latin_ctrl_key_with((KeyCode::Char('Ы'), shift), russian), Some((KeyCode::Char('S'), shift)));
    assert_eq!(latin_ctrl_key_with(ctrl('Ы'), russian), Some(ctrl('S')));
}

#[test]
fn only_ctrl_without_alt_is_mapped() {
    assert_eq!(latin_ctrl_key_with((KeyCode::Char('ы'), KeyModifiers::NONE), russian), None);
    assert_eq!(latin_ctrl_key_with((KeyCode::Char('ы'), KeyModifiers::ALT), russian), None);
    // Ctrl+Alt is AltGr on Windows: a character, not a chord.
    assert_eq!(latin_ctrl_key_with((KeyCode::Char('ы'), CTRL | KeyModifiers::ALT), russian), None);
}

#[test]
fn only_the_letter_keys_are_mapped() {
    // `х` is on VK_OEM_4, where English has `[`: out of scope.
    assert_eq!(latin_ctrl_key_with(ctrl('х'), russian), None);
    // A character no layout has a key for.
    assert_eq!(latin_ctrl_key_with(ctrl('ж'), russian), None);
}

#[test]
fn a_latin_layout_is_unchanged() {
    for c in ['a', 's', 'v', 'b', 'c', 'Z'] {
        assert_eq!(latin_ctrl_key_with(ctrl(c), latin), None, "{c:?}");
    }
    // An ASCII key is what it says it is, whatever the lookup answers.
    let lies = |_| Some(0x53u16);
    assert_eq!(latin_ctrl_key_with(ctrl('v'), lies), None);
}

#[test]
fn a_turkish_q_layout_is_unchanged() {
    // `ı` is a Latin script letter on VK_I: mapping it would turn Ctrl+ı into
    // C-i, which is Tab.
    assert_eq!(latin_ctrl_key_with(ctrl('ı'), turkish_q), None);
    assert_eq!(latin_ctrl_key_with(ctrl('ş'), turkish_q), None);
    assert_eq!(latin_ctrl_key_with(ctrl('ł'), |_| Some(0x4C)), None, "Polish ł on VK_L");
}

#[test]
fn the_send_key_name_is_the_one_an_english_layout_sends() {
    assert_eq!(latin_ctrl_name_with("C-м", russian).as_deref(), Some("C-v"));
    assert_eq!(latin_ctrl_name_with("C-Ы", russian).as_deref(), Some("C-s"));
    assert_eq!(latin_ctrl_name_with("C-S-Ы", russian).as_deref(), Some("C-S-s"));
    assert_eq!(latin_ctrl_name_with("C-v", russian), None);
    assert_eq!(latin_ctrl_name_with("M-ф", russian), None);
    assert_eq!(latin_ctrl_name_with("enter", russian), None);
}

// ── binding lookup (the attached client) ─────────────────────────────────

type Bind = (&'static str, &'static str, &'static str);

fn lookup<'a>(binds: &'a [Bind], tables: &[&str], typed: char) -> Option<&'a str> {
    let typed = ctrl(typed);
    let keys: Vec<_> = std::iter::once(typed).chain(latin_ctrl_key_with(typed, russian)).collect();
    find_binding(binds, tables, &keys, |b| Some((b.0, b.1))).map(|b| b.2)
}

#[test]
fn an_exact_binding_wins() {
    let binds = [("root", "C-s", "latin"), ("root", "C-ы", "exact")];
    assert_eq!(lookup(&binds, &["root"], 'ы'), Some("exact"));
}

#[test]
fn an_unbound_key_falls_back_to_the_latin_binding() {
    let binds = [("root", "C-s", "latin")];
    assert_eq!(lookup(&binds, &["root"], 'ы'), Some("latin"));
    assert_eq!(lookup(&binds, &["prefix"], 'ы'), None, "another table");
    // Prefix pressed twice on Russian: Ctrl+и reaches the default
    // `C-b send-prefix` of the prefix table.
    let binds = [("prefix", "C-b", "send-prefix")];
    assert_eq!(lookup(&binds, &["prefix"], 'и'), Some("send-prefix"));
}

#[test]
fn the_exact_name_wins_across_a_latched_table_and_its_fallback() {
    // A latched table falls back to root; the exact name is tried in both
    // before the Latin key is tried in either.
    let binds = [("mytbl", "C-s", "latin"), ("root", "C-ы", "exact")];
    assert_eq!(lookup(&binds, &["mytbl", "root"], 'ы'), Some("exact"));
}

#[test]
fn a_skipped_binding_does_not_fire() {
    let binds = [("root", "C-s", "latin")];
    let keys = [ctrl('ы'), ctrl('s')];
    assert!(find_binding(&binds, &["root"], &keys, |_| None).is_none());
}

#[test]
fn a_latin_key_finds_its_own_binding_only() {
    let binds = [("root", "C-ы", "exact")];
    let keys: Vec<_> = std::iter::once(ctrl('s')).chain(latin_ctrl_key_with(ctrl('s'), russian)).collect();
    assert!(find_binding(&binds, &["root"], &keys, |b| Some((b.0, b.1))).is_none());
}

// ── prefix ───────────────────────────────────────────────────────────────

#[test]
fn ctrl_and_the_letter_on_the_b_key_arms_a_c_b_prefix() {
    let prefix = Some(ctrl('b'));
    assert!(latin_arms_prefix(latin_ctrl_key_with(ctrl('и'), russian), [prefix, None], false));
    assert!(latin_arms_prefix(latin_ctrl_key_with(ctrl('и'), russian), [None, prefix], false), "prefix2");
    assert!(!latin_arms_prefix(latin_ctrl_key_with(ctrl('м'), russian), [prefix, None], false), "the V key");
}

#[test]
fn a_root_binding_on_the_typed_key_keeps_it_from_arming_the_prefix() {
    let prefix = Some(ctrl('b'));
    assert!(!latin_arms_prefix(latin_ctrl_key_with(ctrl('и'), russian), [prefix, None], true));
}

#[test]
fn a_latin_layout_leaves_the_prefix_check_as_it_was() {
    // Ctrl+B on English is matched by the existing comparison; the Latin
    // rule adds nothing.
    let prefix = Some(ctrl('b'));
    assert!(!latin_arms_prefix(latin_ctrl_key_with(ctrl('b'), latin), [prefix, None], false));
    assert!(!latin_arms_prefix(latin_ctrl_key_with(ctrl('ı'), turkish_q), [Some(ctrl('i')), None], false));
}

// ── delivery to the pane (the server) ────────────────────────────────────

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<u8>>>);

impl crate::pane::PaneInputSink for Recorder {}

impl std::io::Write for Recorder {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Recorder {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

// The pane fixture of test_issue879_ctrl_bracket_escape.rs: a VT byte
// reader with a recording writer, no console, process or server.
fn fixture(host_requests_win32: bool) -> (AppState, Recorder) {
    let (master, _) = crate::util::stub_pane_pty(portable_pty::PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    });
    let mut parser = vt100::Parser::new(24, 80, 0);
    if host_requests_win32 {
        parser.process(b"\x1b[?9001h");
    }
    let recorder = Recorder::default();
    let epoch = Instant::now() - Duration::from_secs(2);
    let pane = Pane {
        master,
        writer: Box::new(recorder.clone()),
        child: crate::util::StubChild::exited(),
        term: Arc::new(Mutex::new(parser)),
        last_rows: 24,
        last_cols: 80,
        id: 1,
        title: String::new(),
        title_locked: false,
        child_pid: None,
        data_version: Arc::new(AtomicU64::new(0)),
        last_title_check: epoch,
        last_infer_title: epoch,
        dead: false,
        dead_info: None,
        last_text_input: None,
        last_special_key: None,
        vt_bridge_cache: None,
        vti_mode_cache: None,
        mouse_input_cache: Some((Instant::now(), Some(0x0208))),
        win32_input_latched: false,
        input_off: false,
        scroll_fg_cache: None,
        mouse_proto_owner: None,
        wheel_auth: None,
        cursor_shape: Arc::new(AtomicU8::new(0)),
        bell_pending: Arc::new(AtomicBool::new(false)),
        cpr_pending: Arc::new(AtomicBool::new(false)),
        color_query_pending: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        copy_state: None,
        live_term: None,
        pane_style: None,
        pane_options: Default::default(),
        squelch_until: None,
        output_ring: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        spawned_at: None,
        start_command: String::new(),
        cwd_hint: None,
    };
    let mut app = AppState::new("ctrl-non-latin-test".to_string());
    app.windows.push(Window {
        root: Node::Leaf(pane),
        active_path: vec![],
        name: "test".to_string(),
        id: 0,
        area: ratatui::layout::Rect::new(0, 0, 80, 24),
        window_size: None,
        window_options: Default::default(),
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: vec![],
        zoom_saved: None,
        linked_from: None,
        floating: vec![],
        floating_focus: None,
    });
    app.active_idx = 0;
    (app, recorder)
}

fn pane(app: &mut AppState) -> &mut Pane {
    let window = &mut app.windows[0];
    crate::tree::active_pane_mut(&mut window.root, &window.active_path).unwrap()
}

fn delivered(key: &str, layout: fn(char) -> Option<u16>, host_requests_win32: bool, latched: bool) -> Vec<u8> {
    let (mut app, recorder) = fixture(host_requests_win32);
    pane(&mut app).win32_input_latched = latched;
    send_key_to_active_with(&mut app, key, layout).unwrap();
    recorder.take()
}

#[test]
fn an_unbound_ctrl_cyrillic_letter_reaches_the_pane_as_ctrl_and_the_latin_letter() {
    for (typed, english) in [("C-м", "C-v"), ("C-ы", "C-s"), ("C-Ы", "C-s"), ("C-S-ы", "C-S-s")] {
        for (host, latched) in [(false, false), (true, false), (true, true)] {
            let want = delivered(english, latin, host, latched);
            assert!(!want.is_empty(), "{english} writes something");
            assert_eq!(delivered(typed, russian, host, latched), want,
                "BUG: {typed} must write what {english} writes (host win32 {host}, latched {latched})");
        }
    }
    assert_eq!(delivered("C-м", russian, false, false), [0x16]);
}

#[test]
fn ctrl_c_keeps_its_byte_and_the_cyrillic_c_key_takes_the_same_route() {
    assert_eq!(delivered("C-c", latin, false, false), [0x03]);
    assert_eq!(delivered("C-c", russian, false, false), [0x03]);
    assert_eq!(delivered("C-с", russian, false, false), [0x03]);
}

#[test]
fn keys_outside_the_rule_are_delivered_as_before() {
    // A Turkish Q `ı` and a Cyrillic letter off the letter keys write what
    // they wrote before: nothing.
    assert_eq!(delivered("C-ı", turkish_q, false, false), b"");
    assert_eq!(delivered("C-х", russian, false, false), b"");
    assert_eq!(delivered("C-w", russian, false, false), [0x17]);
}

// ── copy mode (the server's own key table) ───────────────────────────────

fn copy_app() -> AppState {
    let mut a = AppState::new("ctrl-non-latin-copy".to_string());
    a.window_base_index = 0;
    a.mode_keys = "vi".to_string();
    a.mode = Mode::CopyMode;
    a
}

fn bind(app: &mut AppState, key: &str, value: &str) {
    parse_config_line(app, &format!("bind -T copy-mode-vi {key} set -g @k {value}"));
}

fn ran(app: &AppState) -> Option<&str> {
    app.user_options.get("@k").map(String::as_str)
}

#[test]
fn copy_mode_takes_the_exact_binding_first() {
    let mut a = copy_app();
    bind(&mut a, "C-s", "latin");
    bind(&mut a, "C-ы", "exact");
    send_key_to_active_with(&mut a, "C-ы", russian).unwrap();
    assert_eq!(ran(&a), Some("exact"));
}

#[test]
fn copy_mode_falls_back_to_the_latin_binding() {
    let mut a = copy_app();
    bind(&mut a, "C-s", "latin");
    send_key_to_active_with(&mut a, "C-ы", russian).unwrap();
    assert_eq!(ran(&a), Some("latin"));
}
