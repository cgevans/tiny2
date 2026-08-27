// SPDX-License-Identifier: EUPL-1.2

//! Dynamic V4L2 control mappings for the Tiny 2 vendor extension unit.
//! The wire-format notes in this file are LLM-drafted (Claude).
//!
//! The uvcvideo driver can map fields of UVC extension-unit controls to
//! new V4L2 controls at runtime (UVCIOC_CTRL_MAP). The Tiny 2 takes most
//! commands as writes of up to 4 bytes to selector 0x6, so they can be
//! expressed as V4L2 menu controls whose entry values are the whole
//! command word. Once installed, any V4L2 application can use them.
//!
//! Caveats:
//!
//! * Mappings live in the driver until the device is unplugged; there is
//!   no unmap ioctl. Re-applying is idempotent (EEXIST comes back as
//!   [`ApplyOutcome::AlreadyMapped`]).
//! * Reading a mapped control back returns bytes from the selector's
//!   status layout, not its command layout, so the "current value" V4L2
//!   applications display is not meaningful. Setting works; use
//!   Camera::get_status for real state.
//! * The driver writes the whole 60-byte control on set, so the trailing
//!   bytes are status read-back, not zeros; the camera accepts this.

use crate::usbio::{XuCtrlMap, UVC_CTRL_DATA_TYPE_ENUM, V4L2_CTRL_TYPE_MENU};
use crate::{Camera, Error, TAG_AI_MODE, TAG_FACE_AE, TAG_FOV, TAG_HDR, TAG_LED, TAG_MIRROR, XU_GUID, XU_SEL_COMMAND};

/// Encode a 3-byte selector-0x6 command as a little-endian 24-bit value.
pub const fn cmd3(b0: u8, b1: u8, b2: u8) -> u32 {
    b0 as u32 | (b1 as u32) << 8 | (b2 as u32) << 16
}

/// Encode a 4-byte selector-0x6 command as a little-endian 32-bit value.
pub const fn cmd4(b0: u8, b1: u8, b2: u8, b3: u8) -> u32 {
    cmd3(b0, b1, b2) | (b3 as u32) << 24
}

/// Custom V4L2 control ids, following the uvcdynctrl convention for
/// vendor XU mappings: 0x0A000000 | (USB vendor id << 8) | index.
pub const CID_BASE: u32 = 0x0A35_6410;
pub const CID_AI_TRACKING: u32 = CID_BASE;
pub const CID_HDR: u32 = CID_BASE + 1;
pub const CID_FOV: u32 = CID_BASE + 2;
pub const CID_FACE_AE: u32 = CID_BASE + 3;
pub const CID_MIRROR: u32 = CID_BASE + 4;
pub const CID_LED: u32 = CID_BASE + 5;

/// One command family mapped as a V4L2 menu control.
#[derive(Debug, Clone, Copy)]
pub struct MenuMapping {
    pub cid: u32,
    pub key: &'static str,
    pub name: &'static str,
    pub selector: u8,
    /// Width of the mapped field in bits (24 or 32); offset is always 0.
    pub size_bits: u8,
    pub entries: &'static [(&'static str, u32)],
}

pub const AI_TRACKING: MenuMapping = MenuMapping {
    cid: CID_AI_TRACKING,
    key: "tracking",
    name: "OBSBot AI Tracking Mode",
    selector: XU_SEL_COMMAND,
    size_bits: 32,
    entries: &[
        ("No Tracking", cmd4(TAG_AI_MODE, 0x02, 0, 0)),
        ("Normal Tracking", cmd4(TAG_AI_MODE, 0x02, 2, 0)),
        ("Upper Body", cmd4(TAG_AI_MODE, 0x02, 2, 1)),
        ("Close-up", cmd4(TAG_AI_MODE, 0x02, 2, 2)),
        ("Headless", cmd4(TAG_AI_MODE, 0x02, 2, 3)),
        ("Lower Body", cmd4(TAG_AI_MODE, 0x02, 2, 4)),
        ("Desk Mode", cmd4(TAG_AI_MODE, 0x02, 5, 0)),
        ("Whiteboard", cmd4(TAG_AI_MODE, 0x02, 4, 0)),
        ("Hand", cmd4(TAG_AI_MODE, 0x02, 3, 0)),
        ("Group", cmd4(TAG_AI_MODE, 0x02, 1, 0)),
    ],
};

pub const HDR: MenuMapping = MenuMapping {
    cid: CID_HDR,
    key: "hdr",
    name: "OBSBot HDR",
    selector: XU_SEL_COMMAND,
    size_bits: 24,
    entries: &[
        ("Off", cmd3(TAG_HDR, 0x01, 0)),
        ("On", cmd3(TAG_HDR, 0x01, 1)),
    ],
};

pub const FOV: MenuMapping = MenuMapping {
    cid: CID_FOV,
    key: "fov",
    name: "OBSBot Field of View",
    selector: XU_SEL_COMMAND,
    size_bits: 24,
    entries: &[
        ("Wide (86 deg)", cmd3(TAG_FOV, 0x01, 0)),
        ("Medium (78 deg)", cmd3(TAG_FOV, 0x01, 1)),
        ("Narrow (65 deg)", cmd3(TAG_FOV, 0x01, 2)),
    ],
};

/// Only meaningful while the camera is in auto-exposure mode.
pub const FACE_AE: MenuMapping = MenuMapping {
    cid: CID_FACE_AE,
    key: "face-ae",
    name: "OBSBot Face Auto-Exposure",
    selector: XU_SEL_COMMAND,
    size_bits: 24,
    entries: &[
        ("Off", cmd3(TAG_FACE_AE, 0x01, 0)),
        ("On", cmd3(TAG_FACE_AE, 0x01, 1)),
    ],
};

pub const MIRROR: MenuMapping = MenuMapping {
    cid: CID_MIRROR,
    key: "mirror",
    name: "OBSBot Mirror",
    selector: XU_SEL_COMMAND,
    size_bits: 24,
    entries: &[
        ("Off", cmd3(TAG_MIRROR, 0x01, 0)),
        ("On", cmd3(TAG_MIRROR, 0x01, 1)),
    ],
};

/// The two states are solid (0) and blinking (1), not off and on;
/// write-only, the camera reports no LED mode back.
pub const LED: MenuMapping = MenuMapping {
    cid: CID_LED,
    key: "led",
    name: "OBSBot LED Mode",
    selector: XU_SEL_COMMAND,
    size_bits: 24,
    entries: &[
        ("Solid", cmd3(TAG_LED, 0x01, 0)),
        ("Blinking", cmd3(TAG_LED, 0x01, 1)),
    ],
};

pub const ALL: &[&MenuMapping] = &[&AI_TRACKING, &HDR, &FOV, &FACE_AE, &MIRROR, &LED];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Installed,
    /// A mapping with this control id already existed (kept unchanged).
    AlreadyMapped,
}

impl MenuMapping {
    /// Find a menu entry by label (case-insensitive; '-', '_' and spaces
    /// are equivalent), or by bare entry index. Returns (menu index, label).
    pub fn find_entry(&self, key: &str) -> Option<(i32, &'static str)> {
        if let Ok(idx) = key.parse::<usize>() {
            return self
                .entries
                .get(idx)
                .map(|(label, _)| (idx as i32, *label));
        }
        let norm = |s: &str| -> String {
            s.chars()
                .filter_map(|c| match c {
                    '-' | '_' | ' ' => None,
                    '(' | ')' => None,
                    c => Some(c.to_ascii_lowercase()),
                })
                .collect()
        };
        let key = norm(key);
        self.entries
            .iter()
            .enumerate()
            .find(|(_, (label, _))| {
                let l = norm(label);
                l == key || l.starts_with(&key)
            })
            .map(|(i, (label, _))| (i as i32, *label))
    }
}

impl Camera {
    pub fn apply_mapping(&self, m: &MenuMapping) -> Result<ApplyOutcome, Error> {
        let map = XuCtrlMap {
            id: m.cid,
            name: m.name,
            entity: XU_GUID,
            selector: m.selector,
            size_bits: m.size_bits,
            offset_bits: 0,
            v4l2_type: V4L2_CTRL_TYPE_MENU,
            data_type: UVC_CTRL_DATA_TYPE_ENUM,
            menu: m.entries,
        };
        match self.handle().map_ctrl(&map)? {
            true => Ok(ApplyOutcome::Installed),
            false => Ok(ApplyOutcome::AlreadyMapped),
        }
    }

    pub fn apply_all_mappings(&self) -> Vec<(&'static MenuMapping, Result<ApplyOutcome, Error>)> {
        ALL.iter().map(|m| (*m, self.apply_mapping(m))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AIMode;

    #[test]
    fn ai_tracking_matches_ai_mode_codes() {
        // The menu table must stay in lockstep with AIMode::ALL.
        assert_eq!(AI_TRACKING.entries.len(), AIMode::ALL.len());
        for (mode, (label, value)) in AIMode::ALL.iter().zip(AI_TRACKING.entries) {
            assert_eq!(*label, mode.name());
            let (m, n) = mode.code();
            assert_eq!(*value, cmd4(TAG_AI_MODE, 0x02, m, n), "entry {}", label);
        }
        // Menu index == the integer the OSC API uses.
        for i in 0..AIMode::ALL.len() {
            assert_eq!(AIMode::try_from(i as i32).unwrap(), AIMode::ALL[i]);
        }
    }

    #[test]
    fn family_lockstep_with_setters() {
        use crate::FOVMode;
        for ((label, value), mode) in FOV
            .entries
            .iter()
            .zip([FOVMode::Wide, FOVMode::Normal, FOVMode::Narrow])
        {
            assert_eq!(*value, cmd3(TAG_FOV, 0x01, mode.to_cmd_value()));
            assert!(
                label.contains(&mode.degrees().to_string()),
                "label {} should name {} deg",
                label,
                mode.degrees()
            );
        }
        // Boolean families: entry index == wire value.
        for (mapping, tag) in [
            (&HDR, TAG_HDR),
            (&FACE_AE, TAG_FACE_AE),
            (&MIRROR, TAG_MIRROR),
            (&LED, TAG_LED),
        ] {
            assert_eq!(mapping.entries.len(), 2);
            for (i, (_, value)) in mapping.entries.iter().enumerate() {
                assert_eq!(*value, cmd3(tag, 0x01, i as u8));
            }
        }
    }

    #[test]
    fn command_encoding() {
        assert_eq!(cmd3(0x01, 0x01, 0x01), 0x0001_0101);
        assert_eq!(cmd4(0x16, 0x02, 0x02, 0x04), 0x0402_0216);
    }

    #[test]
    fn cids_unique_and_names_fit() {
        let mut seen = std::collections::HashSet::new();
        let mut seen_keys = std::collections::HashSet::new();
        for m in ALL {
            assert!(seen.insert(m.cid), "duplicate CID {:#x}", m.cid);
            assert!(seen_keys.insert(m.key), "duplicate key {}", m.key);
            assert!(m.name.len() < 32, "control name too long: {}", m.name);
            assert!(matches!(m.size_bits, 24 | 32));
            let mut labels = std::collections::HashSet::new();
            for (label, _) in m.entries {
                assert!(label.len() < 32, "menu label too long: {}", label);
                assert!(labels.insert(*label), "duplicate label {}", label);
            }
            assert!(!m.entries.is_empty());
        }
    }

    #[test]
    fn find_entry_forms() {
        assert_eq!(AI_TRACKING.find_entry("upper-body"), Some((2, "Upper Body")));
        assert_eq!(AI_TRACKING.find_entry("UPPER_BODY"), Some((2, "Upper Body")));
        assert_eq!(AI_TRACKING.find_entry("2"), Some((2, "Upper Body")));
        assert_eq!(HDR.find_entry("on"), Some((1, "On")));
        assert_eq!(FOV.find_entry("wide"), Some((0, "Wide (86 deg)")));
        assert_eq!(AI_TRACKING.find_entry("nonexistent"), None);
        // Prefix matching
        assert_eq!(AI_TRACKING.find_entry("desk"), Some((6, "Desk Mode")));
    }
}
