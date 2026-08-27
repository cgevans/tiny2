// SPDX-License-Identifier: EUPL-1.2

//! Checksummed vendor frame protocol on XU selector 0x2. The wire-format
//! notes in this file are LLM-drafted (Claude).
//!
//! Frames are 60 bytes, zero padded. Layout (all little-endian):
//!
//! ```text
//! off 0   0xAA magic
//! off 1   flags: 0x25 = SET with payload, 0x01 = header-only GET
//!         (replies come back with 0x29)
//! off 2   seq u16
//! off 4   header span = 12 (constant)
//! off 6   header CRC-16/USB over bytes [0,12) with this field zeroed
//! off 8   sender (host = 0x0A)
//! off 9   receiver subsystem (0x02 camera, 0x03 gimbal, 0x04 AI, ...)
//! off 10  command id u16
//! --- only when a payload is present ---
//! off 12  payload length u16
//! off 14  payload CRC-16/USB over [12, 16+len) with this field zeroed
//! off 16  payload bytes
//! ```
//!
//! A SET payload must have exactly the length the firmware expects for
//! that command or it is discarded; GETs are answered only with flags
//! 0x01. The reply mailbox keeps the previous reply until a new one
//! arrives, so replies are matched on (cmd, seq).
//!
//! Only the commands in this module are safe to send: the device also has
//! upgrade/factory subsystems where a stray command can knock it off the
//! bus.

/// Subsystem addresses (frame receiver byte).
pub mod receiver {
    pub const CAMERA: u8 = 0x02;
    pub const GIMBAL: u8 = 0x03;
    pub const AI: u8 = 0x04;
    pub const UPGRADE: u8 = 0x0D;
}

/// Command ids as (wire command, receiver subsystem).
pub mod cmd {
    /// Wake (payload u32 = 0) or sleep (payload u32 = 1).
    pub const CAM_SET_DEV_STATUS: (u16, u8) = (0xA0C2, super::receiver::CAMERA);
    /// Set exposure: payload [mode: u8, value: u32]; mode 1 = manual
    /// (value = shutter time in 0.1 ms), mode 2 = auto (value = AE level).
    pub const CAM_SET_EXPOSURE: (u16, u8) = (0x2982, super::receiver::CAMERA);
    /// Get exposure: reply payload [mode: u8, value: u32].
    pub const CAM_GET_EXPOSURE: (u16, u8) = (0x2942, super::receiver::CAMERA);
    /// Get the auto-sleep delay in seconds (reply payload u32).
    pub const CAM_GET_SUSPEND_TIME: (u16, u8) = (0xA182, super::receiver::CAMERA);
    /// Serial number (reply payload: 14 ASCII bytes).
    pub const UG_GET_SN: (u16, u8) = (0x18C8, super::receiver::UPGRADE);
    /// Tracking responsiveness: payload u8, 0 = standard, 2 = sport.
    pub const AI_SET_TRACK_MODE: (u16, u8) = (0x0CC4, super::receiver::AI);
    /// Drive the gimbal at a speed: payload 3 x f32 [roll, pitch, yaw] in
    /// degrees/second (slot order matters; zeros stop the motion).
    pub const AI_SET_GIM_SPEED: (u16, u8) = (0x6484, super::receiver::AI);
    /// Move the gimbal to absolute motor angles: payload 3 x f32
    /// [roll, pitch, yaw] in degrees.
    pub const AI_SET_GIM_MOTOR_DEG: (u16, u8) = (0x6444, super::receiver::AI);
    /// Recenter the gimbal: payload 6 zero bytes.
    pub const GIM_SET_MOTOR: (u16, u8) = (0x00C3, super::receiver::GIMBAL);
    /// Read gimbal state (24-byte reply; see Camera::gimbal_attitude).
    pub const AI_GET_GIM_STATE: (u16, u8) = (0x6604, super::receiver::AI);
    /// Zoom to an absolute ratio at a speed: payload [speed: u32,
    /// ratio x 100: u32] (speed first).
    pub const CAM_SET_ZOOM_ABSOLUTE: (u16, u8) = (0x1942, super::receiver::CAMERA);
    /// Stop an in-progress zoom (empty payload).
    pub const CAM_SET_ZOOM_STOP: (u16, u8) = (0x19C2, super::receiver::CAMERA);

    /// Gimbal presets (three slots). Reads go through the flat XU
    /// selectors 0x0C/0x0D; the device does not answer framed GETs here.
    /// Preset pose write: payload [index: u32, yaw, pitch, roll, zoom: f32,
    /// -1000.0: f32]. Updates an occupied slot; it cannot create one.
    pub const AI_SET_GIMBAL_PRESET_ADD: (u16, u8) = (0x3944, super::receiver::AI);
    /// Update a preset: same payload as above.
    pub const AI_SET_PRESET_UPDATE_ONLY: (u16, u8) = (0x3E04, super::receiver::AI);
    /// Recall a preset: payload [index: u32, 4 x f32 = 1.0].
    pub const AI_SET_GIMBAL_PRESET_TRIG: (u16, u8) = (0x39C4, super::receiver::AI);
    /// Delete a preset: payload [index: u32].
    pub const AI_SET_GIMBAL_PRESET_DELETE: (u16, u8) = (0x3984, super::receiver::AI);
    /// Rename a preset: payload [index: u32, ASCII name].
    pub const AI_SET_GIMBAL_PRESET_ID_NAME: (u16, u8) = (0x3A84, super::receiver::AI);

    /// Boot pose: where the gimbal points at power-on.
    /// Set: payload [0: u32, yaw, pitch, roll, zoom: f32, 0.0: f32]; all
    /// 24 bytes are required or the write is discarded.
    pub const AI_SET_GIM_BOOT_POS: (u16, u8) = (0x3844, super::receiver::AI);
    /// Get: 24-byte reply in the same layout.
    pub const AI_GET_GIM_BOOT_POS: (u16, u8) = (0x3884, super::receiver::AI);
    /// Reset to the factory boot pose (empty payload).
    pub const AI_RST_GIM_BOOT_POS: (u16, u8) = (0x38C4, super::receiver::AI);
    /// Move to the boot pose now (empty payload).
    pub const AI_TRG_GIM_BOOT_POS: (u16, u8) = (0x3904, super::receiver::AI);
}

pub const FLAGS_SET: u8 = 0x25;
pub const FLAGS_GET: u8 = 0x01;
pub const SENDER_HOST: u8 = 0x0A;
pub const FRAME_LEN: usize = 60;
pub const MAX_PAYLOAD: usize = FRAME_LEN - 16;

/// CRC-16/USB (polynomial 0xA001, init 0xFFFF, xorout 0xFFFF).
pub fn crc16_usb(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub flags: u8,
    pub seq: u16,
    pub sender: u8,
    pub receiver: u8,
    pub cmd: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame too short: {0} bytes")]
    TooShort(usize),
    #[error("bad magic byte {0:#04x}")]
    BadMagic(u8),
    #[error("header CRC mismatch: got {got:#06x}, want {want:#06x}")]
    HeaderCrc { got: u16, want: u16 },
    #[error("payload CRC mismatch: got {got:#06x}, want {want:#06x}")]
    PayloadCrc { got: u16, want: u16 },
    #[error("truncated payload: header says {want} bytes, frame has {got}")]
    Truncated { want: usize, got: usize },
    #[error("payload too long: {0} bytes (max {MAX_PAYLOAD})")]
    PayloadTooLong(usize),
}

pub fn build(flags: u8, seq: u16, (cmd, recv): (u16, u8), payload: &[u8]) -> Result<[u8; FRAME_LEN], FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::PayloadTooLong(payload.len()));
    }
    let mut f = [0u8; FRAME_LEN];
    f[0] = 0xAA;
    f[1] = flags;
    f[2..4].copy_from_slice(&seq.to_le_bytes());
    f[4..6].copy_from_slice(&12u16.to_le_bytes());
    f[8] = SENDER_HOST;
    f[9] = recv;
    f[10..12].copy_from_slice(&cmd.to_le_bytes());
    let hdr_crc = crc16_usb(&f[..12]);
    f[6..8].copy_from_slice(&hdr_crc.to_le_bytes());

    if !payload.is_empty() {
        let len = payload.len();
        f[12..14].copy_from_slice(&(len as u16).to_le_bytes());
        f[16..16 + len].copy_from_slice(payload);
        let seg_crc = crc16_usb(&f[12..16 + len]);
        f[14..16].copy_from_slice(&seg_crc.to_le_bytes());
    }
    Ok(f)
}

pub fn parse(buf: &[u8]) -> Result<Frame, FrameError> {
    if buf.len() < 12 {
        return Err(FrameError::TooShort(buf.len()));
    }
    if buf[0] != 0xAA {
        return Err(FrameError::BadMagic(buf[0]));
    }
    let mut hdr = [0u8; 12];
    hdr.copy_from_slice(&buf[..12]);
    hdr[6] = 0;
    hdr[7] = 0;
    let want = crc16_usb(&hdr);
    let got = u16::from_le_bytes([buf[6], buf[7]]);
    if got != want {
        return Err(FrameError::HeaderCrc { got, want });
    }

    let mut payload = Vec::new();
    if buf.len() >= 16 {
        let len = u16::from_le_bytes([buf[12], buf[13]]) as usize;
        if len > 0 {
            let end = 16 + len;
            if buf.len() < end {
                return Err(FrameError::Truncated {
                    want: len,
                    got: buf.len().saturating_sub(16),
                });
            }
            let mut seg = buf[12..end].to_vec();
            seg[2] = 0;
            seg[3] = 0;
            let want = crc16_usb(&seg);
            let got = u16::from_le_bytes([buf[14], buf[15]]);
            if got != want {
                return Err(FrameError::PayloadCrc { got, want });
            }
            payload = buf[16..end].to_vec();
        }
    }

    Ok(Frame {
        flags: buf[1],
        seq: u16::from_le_bytes([buf[2], buf[3]]),
        sender: buf[8],
        receiver: buf[9],
        cmd: u16::from_le_bytes([buf[10], buf[11]]),
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden frames captured from hardware: the exposure payloads this
    // library used to send verbatim, and a wake frame.
    const AUTO_EXP: [u8; 18] = [
        0xaa, 0x25, 0x16, 0x00, 0x0c, 0x00, 0x58, 0x91, 0x0a, 0x02, 0x82, 0x29, 0x05, 0x00, 0xb2,
        0xaf, 0x02, 0x04,
    ];
    const MANUAL_EXP: [u8; 18] = [
        0xaa, 0x25, 0x15, 0x00, 0x0c, 0x00, 0xa8, 0x9e, 0x0a, 0x02, 0x82, 0x29, 0x05, 0x00, 0xf9,
        0x27, 0x01, 0x32,
    ];
    const WAKE: [u8; 20] = [
        0xaa, 0x25, 0xa5, 0x00, 0x0c, 0x00, 0x5f, 0xef, 0x0a, 0x02, 0xc2, 0xa0, 0x04, 0x00, 0xbe,
        0x07, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn golden_auto_exposure() {
        let f = build(FLAGS_SET, 0x16, cmd::CAM_SET_EXPOSURE, &[0x02, 0x04, 0, 0, 0]).unwrap();
        assert_eq!(&f[..18], &AUTO_EXP);
        assert!(f[18..].iter().all(|&b| b == 0));
    }

    #[test]
    fn golden_manual_exposure() {
        let f = build(FLAGS_SET, 0x15, cmd::CAM_SET_EXPOSURE, &[0x01, 0x32, 0, 0, 0]).unwrap();
        assert_eq!(&f[..18], &MANUAL_EXP);
    }

    #[test]
    fn golden_wake() {
        let f = build(FLAGS_SET, 0xa5, cmd::CAM_SET_DEV_STATUS, &0u32.to_le_bytes()).unwrap();
        assert_eq!(&f[..20], &WAKE);
        assert!(f[20..].iter().all(|&b| b == 0));
    }

    #[test]
    fn roundtrip() {
        let f = build(FLAGS_SET, 0x1234, (0xBEEF, 0x04), &[1, 2, 3, 4, 5, 6, 7]).unwrap();
        let p = parse(&f).unwrap();
        assert_eq!(p.seq, 0x1234);
        assert_eq!(p.cmd, 0xBEEF);
        assert_eq!(p.receiver, 0x04);
        assert_eq!(p.sender, SENDER_HOST);
        assert_eq!(p.flags, FLAGS_SET);
        assert_eq!(p.payload, vec![1, 2, 3, 4, 5, 6, 7]);

        // A header-only GET has no payload segment
        let g = build(FLAGS_GET, 7, cmd::UG_GET_SN, &[]).unwrap();
        let p = parse(&g).unwrap();
        assert_eq!(p.payload, Vec::<u8>::new());
        assert_eq!(p.cmd, cmd::UG_GET_SN.0);
    }

    #[test]
    fn corruption_detected() {
        let mut f = build(FLAGS_SET, 1, cmd::CAM_SET_EXPOSURE, &[2, 4, 0, 0, 0]).unwrap();
        assert!(parse(&f).is_ok());

        let mut bad = f;
        bad[10] ^= 1; // header bit
        assert!(matches!(parse(&bad), Err(FrameError::HeaderCrc { .. })));

        bad = f;
        bad[16] ^= 1; // payload bit
        assert!(matches!(parse(&bad), Err(FrameError::PayloadCrc { .. })));

        bad = f;
        bad[0] = 0;
        assert!(matches!(parse(&bad), Err(FrameError::BadMagic(0))));

        f[12] = 0xFF; // claim a longer payload than the buffer holds
        assert!(matches!(parse(&f), Err(FrameError::Truncated { .. })));

        assert!(matches!(
            build(FLAGS_SET, 0, cmd::CAM_SET_EXPOSURE, &[0; 45]),
            Err(FrameError::PayloadTooLong(45))
        ));
    }

    #[test]
    fn crc_reference_values() {
        // Check value for "123456789" from the CRC catalogue.
        assert_eq!(crc16_usb(b"123456789"), 0xB4C8);
        assert_eq!(crc16_usb(&[]), 0x0000);
    }
}
