// char_to_vk gained explicit non-Win32 match arms for ESC ('\x1b' → 0x1B) and
// Enter ('\r' → 0x0D) so that send_modified_key_event works for those keys
// without VkKeyScanW returning -1 for them.

use super::mouse_inject::char_to_vk;

#[test]
fn escape_returns_vk_escape() {
    assert_eq!(char_to_vk('\x1b'), 0x1Bu16, "VK_ESCAPE");
}

#[test]
fn carriage_return_returns_vk_return() {
    assert_eq!(char_to_vk('\r'), 0x0Du16, "VK_RETURN");
}

#[test]
fn alphabetic_lowercase_maps_to_vk() {
    assert_eq!(char_to_vk('a'), 0x41u16, "VK_A");
    assert_eq!(char_to_vk('c'), 0x43u16, "VK_C");
    assert_eq!(char_to_vk('z'), 0x5Au16, "VK_Z");
}

#[test]
fn latin_letters_and_digits_name_their_key_in_every_layout() {
    // Pure: no VkKeyScanW, so the answer does not follow the active layout
    // (a Russian layout has no key for `a`, VkKeyScanW returns -1 there).
    use super::latin_vk;
    assert_eq!(latin_vk('a'), Some(0x41));
    assert_eq!(latin_vk('L'), Some(0x4C));
    assert_eq!(latin_vk('0'), Some(0x30));
    assert_eq!(latin_vk('ф'), None);
    assert_eq!(latin_vk('['), None);
    assert_eq!(latin_vk('\r'), None);
}

#[test]
fn alphabetic_uppercase_same_as_lowercase() {
    // char_to_vk normalises to lowercase before calling VkKeyScanW
    assert_eq!(char_to_vk('A'), char_to_vk('a'));
    assert_eq!(char_to_vk('Z'), char_to_vk('z'));
}
