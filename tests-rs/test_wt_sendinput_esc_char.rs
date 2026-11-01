// Windows Terminal `sendInput` of ESC + a character while a non Latin layout is
// active (a key bound to "\u001bv", Claude Code's Alt+V).
//
// WT writes the text to the pseudoconsole as is. conhost turns ESC v into an
// Alt keypress through VkKeyScanW('v'), which fails on Russian or Ukrainian
// (0xFFFF), and its LOBYTE/HIBYTE split of the failure is never -1, so the
// record carries vk 0xFF, scan code 0, the character, and all of
// SHIFT | LEFT_CTRL | LEFT_ALT. crossterm read that as C-M-v. The tap decodes
// it as Alt plus the character (ESC v). A key a keyboard sends always has a
// scan code and a real virtual key, so every real chord stays as it was.

use super::*;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

const LCTRL: u32 = 0x0008;
const LALT: u32 = 0x0002;
const RALT: u32 = 0x0001;
const SHIFT: u32 = 0x0010;

fn rec(down: bool, vk: u16, scan: u16, u: u16, ctrl_state: u32) -> Option<KeyRec> {
    Some(KeyRec { key_down: down, vk, scan, u_char: u, ctrl_state })
}

fn key(c: char, modifiers: KeyModifiers, kind: KeyEventKind) -> Verdict {
    Verdict::Consumed(Some(Event::Key(KeyEvent { code: KeyCode::Char(c), modifiers, kind, state: KeyEventState::empty() })))
}

fn russian(r: &KeyRec) -> Option<char> {
    (r.vk == 0x56).then_some('м')
}

/// The pair conhost writes for ESC v on Russian, measured in a pseudoconsole.
#[test]
fn wt_sendinput_esc_v_is_alt_v_press_and_release() {
    let mut tap = ConsoleTap::new();
    let cs = SHIFT | LCTRL | LALT;
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, b'v' as u16, cs), russian), key('v', KeyModifiers::ALT, KeyEventKind::Press));
    assert_eq!(tap.classify_with(rec(false, 0xFF, 0, b'v' as u16, cs), russian), key('v', KeyModifiers::ALT, KeyEventKind::Release));
    // ESC V keeps the case the character carries.
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, b'V' as u16, cs), russian), key('V', KeyModifiers::ALT, KeyEventKind::Press));
}

#[test]
fn real_ctrl_alt_shift_v_stays_crossterms() {
    let mut tap = ConsoleTap::new();
    let cs = SHIFT | LCTRL | LALT;
    assert_eq!(tap.classify_with(rec(true, 0x56, 0x2F, 0x16, cs), russian), Verdict::NotOurs);
    assert_eq!(tap.classify_with(rec(true, 0x56, 0x2F, 0x16, cs), |_| None), Verdict::NotOurs);
    // A real key with no virtual key mapping still has its scan code.
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0x73, b'v' as u16, cs), russian), Verdict::NotOurs);
    // vk 0xFF with a partial modifier set is not the VkKeyScanW failure.
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, b'v' as u16, LCTRL | LALT), russian), Verdict::NotOurs);
    // No character: nothing to send.
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, 0, cs), russian), Verdict::NotOurs);
}

#[test]
fn altgr_text_keeps_its_rules() {
    let mut tap = ConsoleTap::new();
    // German AltGr+Q is '@': crossterm's.
    assert_eq!(tap.classify_with(rec(true, 0x51, 0x10, b'@' as u16, LCTRL | RALT), russian), Verdict::NotOurs);
    // A lowercase ASCII letter under AltGr is still the plain letter.
    assert_eq!(tap.classify_with(rec(true, 0x49, 0x17, b'i' as u16, LCTRL | RALT), russian), key('i', KeyModifiers::empty(), KeyEventKind::Press));
}

#[test]
fn russian_ctrl_v_is_still_ctrl_v() {
    let mut tap = ConsoleTap::new();
    assert_eq!(tap.classify_with(rec(true, 0x56, 0x2F, 0x16, LCTRL), russian), key('v', KeyModifiers::CONTROL, KeyEventKind::Press));
}

#[test]
fn a_missing_modifier_bit_is_not_the_vkkeyscan_failure() {
    let mut tap = ConsoleTap::new();
    for cs in [SHIFT | LALT, SHIFT | LCTRL, LCTRL | LALT] {
        assert_eq!(tap.classify_with(rec(true, 0xFF, 0, b'v' as u16, cs), russian), Verdict::NotOurs, "cs={cs:#x}");
    }
}

#[test]
fn a_control_character_or_del_is_not_taken() {
    let mut tap = ConsoleTap::new();
    let cs = SHIFT | LCTRL | LALT;
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, 0x01, cs), russian), Verdict::NotOurs);
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, 0x7F, cs), russian), Verdict::NotOurs);
}

#[test]
fn punctuation_and_non_ascii_go_on_with_alt() {
    let mut tap = ConsoleTap::new();
    let cs = SHIFT | LCTRL | LALT;
    // ESC @ on Russian: no key types '@' there.
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, b'@' as u16, cs), russian), key('@', KeyModifiers::ALT, KeyEventKind::Press));
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, 'é' as u16, cs), russian), key('é', KeyModifiers::ALT, KeyEventKind::Press));
}

#[test]
fn only_the_press_drops_a_pending_high_surrogate() {
    let mut tap = ConsoleTap::new();
    let cs = SHIFT | LCTRL | LALT;
    let smile = Verdict::Consumed(Some(Event::Key(KeyEvent {
        code: KeyCode::Char('😊'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })));
    // A release between the halves leaves the pair whole.
    assert_eq!(tap.classify_with(rec(true, 0, 0, 0xD83D, 0), russian), Verdict::Consumed(None));
    assert_eq!(tap.classify_with(rec(false, 0xFF, 0, b'v' as u16, cs), russian), key('v', KeyModifiers::ALT, KeyEventKind::Release));
    assert_eq!(tap.classify_with(rec(true, 0, 0, 0xDE0A, 0), russian), smile);
    // A press breaks it.
    assert_eq!(tap.classify_with(rec(true, 0, 0, 0xD83D, 0), russian), Verdict::Consumed(None));
    assert_eq!(tap.classify_with(rec(true, 0xFF, 0, b'v' as u16, cs), russian), key('v', KeyModifiers::ALT, KeyEventKind::Press));
    assert_eq!(tap.classify_with(rec(true, 0, 0, 0xDE0A, 0), russian), Verdict::Consumed(None));
}
