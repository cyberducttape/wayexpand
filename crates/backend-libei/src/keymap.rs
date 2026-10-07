use std::{
    collections::HashMap,
    fs::File,
    io::{Read as _, Seek, SeekFrom},
    os::fd::OwnedFd,
};
use xkbcommon_rs::{
    xkb_state::KeyDirection, Context, Keymap as XkbKeymap, KeymapFormat, State as XkbState,
};

use super::{LibeiError, MAX_KEYMAP_BYTES, XKB_KEYCODE_OFFSET};

/// Modifier keys the keysym fallback may hold around a key, as bits of
/// [`KeyStroke::modifiers`]. Each is a key of the server's own keymap, so the
/// compositor resolves the level exactly as for a physical keyboard.
pub(super) const MOD_SHIFT: u8 = 1 << 0;
pub(super) const MOD_LEVEL3: u8 = 1 << 1;
pub(super) const MOD_LEVEL5: u8 = 1 << 2;
/// The `MOD_*` bits, indexed like [`KeysymTyper::modifier_keycodes`].
pub(super) const MODIFIER_BITS: [u8; 3] = [MOD_SHIFT, MOD_LEVEL3, MOD_LEVEL5];

/// One synthesized key press: an evdev keycode and the modifier keys (bits
/// of `MOD_*`) held around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct KeyStroke {
    pub(super) keycode: u32,
    pub(super) modifiers: u8,
}

/// Maps characters to a keystroke reachable on the EIS server's own keymap,
/// for one active layout group and set of locked modifiers. Rebuilt whenever
/// the server reports a different group or lock state.
pub(super) struct KeysymTyper {
    pub(super) keymap: XkbKeymap,
    /// Layout group and locked-modifier mask the map below was built for.
    pub(super) built_for: KeyboardLock,
    /// Latest group/locks reported by the server (`None` until the first
    /// `ei_keyboard.modifiers` event).
    pub(super) reported: Option<KeyboardLock>,
    /// Evdev keycodes of the Shift, Level3 (AltGr) and Level5 keys, indexed
    /// like the `MOD_*` bits. Level3/Level5 are absent on layouts that do
    /// not define them.
    pub(super) modifier_keycodes: [Option<u32>; 3],
    /// Evdev keycode for Return/Enter key (for newlines).
    pub(super) return_keycode: u32,
    /// Evdev keycode for Tab key (for horizontal tabs).
    pub(super) tab_keycode: u32,
    /// The simplest keystroke producing each character on layout 0.
    pub(super) chars: HashMap<char, KeyStroke>,
}

/// Server-side keyboard state that changes which key produces a character:
/// the active layout group and locked modifiers such as Caps Lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct KeyboardLock {
    group: u32,
    locked_mods: u32,
}

impl KeysymTyper {
    pub(super) fn build(keymap: &XkbKeymap) -> Result<Self, LibeiError> {
        Self::build_for(keymap.clone(), KeyboardLock::default(), None)
    }

    pub(super) fn build_for(
        keymap: XkbKeymap,
        lock: KeyboardLock,
        reported: Option<KeyboardLock>,
    ) -> Result<Self, LibeiError> {
        if lock.group as usize >= keymap.num_layouts() {
            return Err(LibeiError::Keymap(format!(
                "server reported layout group {} but the keymap has {} layouts",
                lock.group,
                keymap.num_layouts()
            )));
        }
        let keymap_ref = &keymap;
        let shift_keycode = find_keycode_for_keysym(keymap_ref, xkeysym::key::Shift_L)
            .or_else(|| find_keycode_for_keysym(keymap_ref, xkeysym::key::Shift_R))
            .ok_or_else(|| LibeiError::Keymap("current keymap has no Shift key".into()))?;
        let return_keycode = find_keycode_for_keysym(keymap_ref, xkeysym::key::Return)
            .ok_or_else(|| LibeiError::Keymap("current keymap has no Return key".into()))?;
        let tab_keycode = find_keycode_for_keysym(keymap_ref, xkeysym::key::Tab)
            .ok_or_else(|| LibeiError::Keymap("current keymap has no Tab key".into()))?;
        let modifier_keycodes = [
            Some(shift_keycode),
            find_keycode_for_keysym(keymap_ref, xkeysym::key::ISO_Level3_Shift),
            find_keycode_for_keysym(keymap_ref, xkeysym::key::ISO_Level5_Shift),
        ];
        // Rather than translating XKB modifier masks back into keys, hold
        // each available combination of the layout's own modifier keys in an
        // XKB state and record what every key then produces. Fewest
        // modifiers first, so the simplest chord wins (e.g. 'a' unmodified,
        // 'A' with Shift, '@' with AltGr on a German layout).
        let mut combinations: Vec<u8> = (0..8u8)
            .filter(|mask| {
                MODIFIER_BITS
                    .iter()
                    .zip(&modifier_keycodes)
                    .all(|(bit, keycode)| mask & bit == 0 || keycode.is_some())
            })
            .collect();
        combinations.sort_by_key(|mask| mask.count_ones());
        let mut chars = HashMap::new();
        for modifiers in combinations {
            let mut state = XkbState::new(keymap.clone());
            // Start from the server's active layout and locks (Caps Lock
            // changes which level an unmodified key produces).
            state.update_mask(0, 0, lock.locked_mods, 0, 0, lock.group as usize);
            for (bit, keycode) in MODIFIER_BITS.iter().zip(&modifier_keycodes) {
                if let Some(keycode) = keycode.filter(|_| modifiers & bit != 0) {
                    state.update_key(keycode + XKB_KEYCODE_OFFSET, KeyDirection::Down);
                }
            }
            for &xkb_keycode in keymap_ref.iter_keycodes() {
                let Some(evdev_keycode) = xkb_keycode.checked_sub(XKB_KEYCODE_OFFSET) else {
                    continue;
                };
                if modifier_keycodes.contains(&Some(evdev_keycode)) {
                    continue;
                }
                for sym in state.key_get_syms(xkb_keycode) {
                    if let Some(character) = sym.key_char() {
                        chars.entry(character).or_insert(KeyStroke {
                            keycode: evdev_keycode,
                            modifiers,
                        });
                    }
                }
            }
        }
        Ok(Self {
            keymap,
            built_for: lock,
            reported,
            modifier_keycodes,
            return_keycode,
            tab_keycode,
            chars,
        })
    }

    /// Record the server's latest group/lock state; the map is rebuilt
    /// lazily by [`KeysymTyper::current`].
    pub(super) fn note_modifiers(&mut self, group: u32, locked_mods: u32) {
        self.reported = Some(KeyboardLock { group, locked_mods });
    }

    /// Bring the character map in line with the server's reported state, or
    /// refuse. With several layouts and no report, the active layout is
    /// unknown and guessing could type wrong characters.
    pub(super) fn refresh(&mut self) -> Result<(), LibeiError> {
        let wanted = match self.reported {
            Some(lock) => lock,
            None if self.keymap.num_layouts() > 1 => {
                return Err(LibeiError::UnknownActiveLayout(self.keymap.num_layouts()))
            }
            None => KeyboardLock::default(),
        };
        if wanted != self.built_for {
            *self = Self::build_for(self.keymap.clone(), wanted, self.reported)?;
        }
        Ok(())
    }

    /// Modifier keycodes to press (in order) around `stroke`, released in
    /// reverse.
    pub(super) fn modifiers_for(
        &self,
        stroke: KeyStroke,
    ) -> impl DoubleEndedIterator<Item = u32> + '_ {
        MODIFIER_BITS
            .iter()
            .zip(&self.modifier_keycodes)
            .filter(move |(bit, _)| stroke.modifiers & **bit != 0)
            .filter_map(|(_, keycode)| *keycode)
    }
}

pub(super) fn find_keycode_for_keysym(
    keymap: &XkbKeymap,
    raw_keysym: xkeysym::RawKeysym,
) -> Option<u32> {
    let target = xkeysym::Keysym::new(raw_keysym);
    keymap.iter_keycodes().find_map(|&xkb_keycode| {
        let syms = keymap.key_get_syms_by_level(xkb_keycode, 0, 0).ok()?;
        syms.contains(&target)
            .then(|| xkb_keycode.checked_sub(XKB_KEYCODE_OFFSET))
            .flatten()
    })
}

pub(super) fn decode_keymap(fd: OwnedFd, size: u32) -> Result<XkbKeymap, LibeiError> {
    if size == 0 || size > MAX_KEYMAP_BYTES {
        return Err(LibeiError::Keymap(format!(
            "invalid keymap size {size} bytes"
        )));
    }
    let mut file = File::from(fd);
    file.seek(SeekFrom::Start(0))
        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let text = String::from_utf8(bytes).map_err(|error| LibeiError::Keymap(error.to_string()))?;
    XkbKeymap::new_from_string(
        Context::new(0).map_err(|error| LibeiError::Keymap(error.to_string()))?,
        &text,
        KeymapFormat::TextV1,
        0,
    )
    .map_err(|error| LibeiError::Keymap(error.to_string()))
}
