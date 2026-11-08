//! Windows virtual-key helpers.

pub const VK_RETURN: u16 = 0x0D;
pub const VK_TAB: u16 = 0x09;

/// Keys that need KEYEVENTF_EXTENDEDKEY so the dedicated (non-numpad) variant is produced.
pub fn is_extended_key(vk: u16) -> bool {
    matches!(
        vk,
        0x03 // VK_CANCEL (Ctrl+Break)
        | 0x21 // PRIOR (PgUp)
        | 0x22 // NEXT (PgDn)
        | 0x23 // END
        | 0x24 // HOME
        | 0x25 // LEFT
        | 0x26 // UP
        | 0x27 // RIGHT
        | 0x28 // DOWN
        | 0x2C // SNAPSHOT (PrtSc)
        | 0x2D // INSERT
        | 0x2E // DELETE
        | 0x5B // LWIN
        | 0x5C // RWIN
        | 0x5D // APPS
        | 0x6F // DIVIDE (numpad /)
        | 0x90 // NUMLOCK
        | 0xA3 // RCONTROL
        | 0xA5 // RMENU (AltGr / right Alt)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_keys_are_extended() {
        for vk in [0x21u16, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E] {
            assert!(is_extended_key(vk), "{vk:#x}");
        }
    }

    #[test]
    fn letters_and_modifiers_are_not_extended() {
        for vk in [0x41u16, 0x5A, 0x30, 0x11, 0x12, 0x10, 0x1B, 0x70, 0x0D, 0x20, 0x09] {
            assert!(!is_extended_key(vk), "{vk:#x}");
        }
        assert!(is_extended_key(0x5B)); // Win
    }
}
