// SPDX-License-Identifier: EUPL-1.2

//! Gimbal presets and the boot pose. The wire-format notes in this file
//! are LLM-drafted (Claude).
//!
//! Preset writes are framed selector-0x2 commands, but the device does not
//! answer framed GETs for them; reads go through two flat selectors:
//!
//! * selector 0x0C: `[len: u8, entry: u8 x len]`. The entries before the
//!   first 0xFF are the occupied slot indices. Echo-writing the block back
//!   resets the entry cursor.
//! * selector 0x0D: an entry cursor; each read returns one preset and
//!   advances; byte 0 is a status (0x02 = list exhausted), then slot index
//!   at 1, pitch and yaw as i16 centidegrees at 4 and 6, zoom x100 at 8,
//!   and a NUL-terminated base64 name from 10.
//!
//! Selector 0x07 holds the pose records: `[count: u8]` then 9-byte records
//! `[id, roll i16, pitch i16, yaw i16, zoom u8]`.
//!
//! Creating a preset in a free slot is not implemented: no decoded command
//! does it. Writes to 0x07 and the framed add command only update the pose
//! of a record that already exists. Deleting a preset is therefore
//! irreversible from Linux.
//!
//! The boot pose (where the gimbal points at power-on) is a separate
//! single global record with its own set/get/reset/trigger commands.

use crate::{frame, Camera, Error, XU_UNIT_ID};

/// Selector holding the preset slot list.
pub const XU_SEL_PRESET_LIST: u8 = 0x0C;
/// Selector serving preset entries cursor-style.
pub const XU_SEL_PRESET_ENTRY: u8 = 0x0D;

pub const SLOTS: u8 = 3;

/// A gimbal pose. Angles in degrees (positive yaw right, positive pitch
/// down); zoom is a ratio (1.0-2.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub zoom: f32,
}

/// One stored preset.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    /// Slot number, 1-based.
    pub slot: u8,
    pub name: String,
    pub pose: Pose,
}

fn check_slot(slot: u8) -> Result<u32, Error> {
    if (1..=SLOTS).contains(&slot) {
        Ok((slot - 1) as u32)
    } else {
        Err(Error::UnsupportedIntValue(
            "preset slot".to_string(),
            slot as i32,
        ))
    }
}

fn pose_bytes(pose: Pose, trailer: f32) -> [u8; 20] {
    let mut b = [0u8; 20];
    b[0..4].copy_from_slice(&pose.yaw.to_le_bytes());
    b[4..8].copy_from_slice(&pose.pitch.to_le_bytes());
    b[8..12].copy_from_slice(&pose.roll.to_le_bytes());
    b[12..16].copy_from_slice(&pose.zoom.to_le_bytes());
    b[16..20].copy_from_slice(&trailer.to_le_bytes());
    b
}

impl Camera {
    pub fn preset_list(&self) -> Result<Vec<Preset>, Error> {
        // Echoing the slot-list block back resets the entry cursor; only a
        // block the device itself returned may be echoed, so an all-zero
        // (failed) read is an error. The first byte is not a reliable
        // count; the entry walk below self-terminates.
        let mut list = [0u8; 60];
        self.get_cur_block(XU_UNIT_ID, XU_SEL_PRESET_LIST, &mut list)?;
        if list.iter().all(|&b| b == 0) {
            return Err(Error::BadPresetList(0));
        }
        let mut echo = list;
        self.set_cur_block(XU_UNIT_ID, XU_SEL_PRESET_LIST, &mut echo)?;

        let mut out: Vec<Preset> = Vec::new();
        for _ in 0..SLOTS {
            let mut entry = [0u8; 60];
            self.get_cur_block(XU_UNIT_ID, XU_SEL_PRESET_ENTRY, &mut entry)?;
            // Stop at an exhausted cursor (0x02), an out-of-range or
            // all-zero header (failed read), or a repeated slot.
            let slot_idx = entry[1];
            if entry[0] == 0x02
                || slot_idx >= SLOTS
                || entry[..10].iter().all(|&b| b == 0)
                || out.iter().any(|p| p.slot == slot_idx + 1)
            {
                break;
            }
            let pitch = i16::from_le_bytes([entry[4], entry[5]]) as f32 / 100.0;
            let yaw = i16::from_le_bytes([entry[6], entry[7]]) as f32 / 100.0;
            let zoom = entry[8] as f32 / 100.0;
            let name_b64: &[u8] = &entry[10..];
            let end = name_b64.iter().position(|&b| b == 0).unwrap_or(name_b64.len());
            // The base64 field is padded with junk; keep the printable
            // ASCII prefix of the decoded name and fall back to empty.
            let name = base64_decode(&name_b64[..end])
                .map(|bytes| {
                    let printable: Vec<u8> = bytes
                        .into_iter()
                        .take_while(|b| (0x20..0x7f).contains(b))
                        .collect();
                    String::from_utf8_lossy(&printable).into_owned()
                })
                .unwrap_or_default();
            out.push(Preset {
                slot: slot_idx + 1,
                name,
                pose: Pose {
                    yaw,
                    pitch,
                    roll: 0.0,
                    zoom,
                },
            });
        }
        Ok(out)
    }

    pub fn preset_recall(&self, slot: u8) -> Result<(), Error> {
        let idx = check_slot(slot)?;
        let mut p = [0u8; 20];
        p[0..4].copy_from_slice(&idx.to_le_bytes());
        for i in 0..4 {
            p[4 + i * 4..8 + i * 4].copy_from_slice(&1.0f32.to_le_bytes());
        }
        self.send_frame(frame::cmd::AI_SET_GIMBAL_PRESET_TRIG, &p)?;
        Ok(())
    }

    /// Overwrite the pose of an existing preset. Only works on an
    /// occupied slot; nothing decoded so far can create a preset in a
    /// free slot, so an empty slot is an error, not a no-op.
    pub fn preset_update(&self, slot: u8, pose: Pose) -> Result<(), Error> {
        let idx = check_slot(slot)?;
        if !self.preset_list()?.iter().any(|p| p.slot == slot) {
            return Err(Error::PresetSlotEmpty(slot));
        }
        let mut p = [0u8; 24];
        p[0..4].copy_from_slice(&idx.to_le_bytes());
        p[4..24].copy_from_slice(&pose_bytes(pose, -1000.0));
        self.send_frame(frame::cmd::AI_SET_PRESET_UPDATE_ONLY, &p)?;
        Ok(())
    }

    /// Delete a stored preset; irreversible from Linux (no create command).
    pub fn preset_delete(&self, slot: u8) -> Result<(), Error> {
        let idx = check_slot(slot)?;
        self.send_frame(frame::cmd::AI_SET_GIMBAL_PRESET_DELETE, &idx.to_le_bytes())?;
        Ok(())
    }

    /// Rename a stored preset (plain ASCII; stored base64 on the camera).
    pub fn preset_set_name(&self, slot: u8, name: &str) -> Result<(), Error> {
        let idx = check_slot(slot)?;
        let mut p = idx.to_le_bytes().to_vec();
        p.extend_from_slice(name.as_bytes());
        self.send_frame(frame::cmd::AI_SET_GIMBAL_PRESET_ID_NAME, &p)?;
        Ok(())
    }

    /// Set the power-on gimbal pose; all 24 payload bytes are required
    /// or the write is discarded.
    pub fn boot_pose_set(&self, pose: Pose) -> Result<(), Error> {
        let mut p = [0u8; 24];
        p[4..24].copy_from_slice(&pose_bytes(pose, 0.0));
        self.send_frame(frame::cmd::AI_SET_GIM_BOOT_POS, &p)?;
        Ok(())
    }

    pub fn boot_pose_get(&self) -> Result<Pose, Error> {
        let p = self.framed_transact(frame::FLAGS_GET, frame::cmd::AI_GET_GIM_BOOT_POS, &[])?;
        if p.len() < 20 {
            return Err(Error::BadFrameReply {
                cmd: frame::cmd::AI_GET_GIM_BOOT_POS.0,
                payload: p,
            });
        }
        let f = |o: usize| f32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
        Ok(Pose {
            yaw: f(4),
            pitch: f(8),
            roll: f(12),
            zoom: f(16),
        })
    }

    pub fn boot_pose_reset(&self) -> Result<(), Error> {
        self.send_frame(frame::cmd::AI_RST_GIM_BOOT_POS, &[])?;
        Ok(())
    }

    pub fn boot_pose_trigger(&self) -> Result<(), Error> {
        self.send_frame(frame::cmd::AI_TRG_GIM_BOOT_POS, &[])?;
        Ok(())
    }
}

/// Minimal base64 decoder (standard alphabet, '=' padding); avoids a
/// dependency for the short preset-name strings.
fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    fn val(b: u8) -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'a'..=b'z' => Some((b - b'a' + 26) as u32),
            b'0'..=b'9' => Some((b - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let input: Vec<u8> = input
        .iter()
        .copied()
        .filter(|&b| b != b'=' && b != b'\n' && b != b'\r')
        .collect();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.chunks(4) {
        let mut acc: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            acc |= val(b)? << (18 - 6 * i);
        }
        let bytes = [(acc >> 16) as u8, (acc >> 8) as u8, acc as u8];
        let n = match chunk.len() {
            2 => 1,
            3 => 2,
            4 => 3,
            _ => return None,
        };
        out.extend_from_slice(&bytes[..n]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(base64_decode(b"UHJlc2V0czE=").unwrap(), b"Presets1");
        assert_eq!(base64_decode(b"").unwrap(), b"");
        assert_eq!(base64_decode(b"TQ==").unwrap(), b"M");
        assert_eq!(base64_decode(b"TWE=").unwrap(), b"Ma");
        assert_eq!(base64_decode(b"TWFu").unwrap(), b"Man");
        assert!(base64_decode(b"!!!").is_none());
    }

    #[test]
    fn slot_validation() {
        assert!(check_slot(0).is_err());
        assert_eq!(check_slot(1).unwrap(), 0);
        assert_eq!(check_slot(3).unwrap(), 2);
        assert!(check_slot(4).is_err());
    }

    #[test]
    fn pose_encoding() {
        let b = pose_bytes(
            Pose {
                yaw: 9.39,
                pitch: -8.2,
                roll: 0.0,
                zoom: 1.0,
            },
            -1000.0,
        );
        assert_eq!(&b[0..4], &9.39f32.to_le_bytes());
        assert_eq!(&b[4..8], &(-8.2f32).to_le_bytes());
        assert_eq!(&b[16..20], &(-1000.0f32).to_le_bytes());
    }

    #[test]
    fn entry_decode_shapes() {
        // Example selector-0x0D block: slot 0, name "Presets1".
        let mut entry = [0u8; 60];
        entry[1] = 0;
        entry[4..6].copy_from_slice(&(-820i16).to_le_bytes());
        entry[6..8].copy_from_slice(&939i16.to_le_bytes());
        entry[8] = 100;
        entry[10..22].copy_from_slice(b"UHJlc2V0czE=");
        let pitch = i16::from_le_bytes([entry[4], entry[5]]) as f32 / 100.0;
        let yaw = i16::from_le_bytes([entry[6], entry[7]]) as f32 / 100.0;
        assert_eq!(pitch, -8.2);
        assert_eq!(yaw, 9.39);
    }
}
