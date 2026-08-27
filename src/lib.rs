// SPDX-License-Identifier: EUPL-1.2
//! The wire-format notes in this file are LLM-drafted (Claude).

mod usbio;
pub mod frame;
pub mod mapping;
pub mod preset;

use nix::errno::Errno;
use std::sync::atomic::{AtomicU16, Ordering};
use std::{fmt::Display, io, thread, time::Duration};
use thiserror::Error;

pub use usbio::DeviceInfo;

/// Vendor extension unit: bUnitID 2, GUID
/// {9a1e7291-6843-4683-6d92-39bc7906ee49} (raw byte order below).
pub const XU_UNIT_ID: u8 = 2;
pub const XU_GUID: [u8; 16] = [
    0x91, 0x72, 0x1e, 0x9a, 0x43, 0x68, 0x83, 0x46, 0x6d, 0x92, 0x39, 0xbc, 0x79, 0x06, 0xee, 0x49,
];

/// XU selector for most commands and the status block.
pub const XU_SEL_COMMAND: u8 = 0x6;

// Selector-0x6 command tags; wire form is [tag, len, value...].
pub(crate) const TAG_HDR: u8 = 0x01;
pub(crate) const TAG_SUSPEND_TIME: u8 = 0x0b;
pub(crate) const TAG_VERTICAL: u8 = 0x0c;
pub(crate) const TAG_NO_SLEEP_WITHOUT_STREAM: u8 = 0x12;
pub(crate) const TAG_MIC_DURING_SLEEP: u8 = 0x13;
pub(crate) const TAG_BOOT_MODE: u8 = 0x19;
pub(crate) const TAG_FACE_AE: u8 = 0x03;
pub(crate) const TAG_FOV: u8 = 0x04;
pub(crate) const TAG_MIRROR: u8 = 0x14;
pub(crate) const TAG_AI_MODE: u8 = 0x16;
pub(crate) const TAG_LED: u8 = 0x18;
/// XU selector for the checksummed frame protocol (see frame.rs).
pub const XU_SEL_EXPOSURE: u8 = 0x2;

#[derive(Error, Debug)]
pub enum Error {
    #[error("value of {1} is not supported for {0}")]
    UnsupportedIntValue(String, i32),
    #[error("USB I/O error: {0}")]
    Usb(#[from] Errno),
    #[error("IO error: {0}")]
    IOError(#[from] io::Error),
    #[error("no camera found")]
    NoCameraFound,
    #[error("device reports control length {expected}, need {got}")]
    BufferSize { expected: usize, got: usize },
    #[error("status block too short: {0} bytes")]
    ShortStatus(usize),
    #[error("unknown AI mode code ({0}, {1})")]
    UnknownAIMode(u8, u8),
    #[error("vendor frame error: {0}")]
    Frame(#[from] frame::FrameError),
    #[error("no reply to framed command {cmd:#06x} (seq {seq})")]
    NoFrameReply { cmd: u16, seq: u16 },
    #[error("unexpected reply payload for {cmd:#06x}: {payload:02x?}")]
    BadFrameReply { cmd: u16, payload: Vec<u8> },
    #[error("implausible preset list (count {0}): read failed or camera asleep")]
    BadPresetList(u8),
    #[error("preset slot {0} is empty, and creating a preset is not supported")]
    PresetSlotEmpty(u8),
}

#[derive(Debug)]
pub struct Camera {
    handle: usbio::CameraHandle,
    verbose: bool,
    seq: AtomicU16,
}

/// Device running state (status byte 0x09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevStatus {
    Run,
    /// Asleep: most commands are ignored, reads still work.
    Sleep,
    /// Privacy mode: no video stream.
    Privacy,
    Unknown(u8),
}

impl DevStatus {
    fn from_byte(b: u8) -> Self {
        match b {
            1 => DevStatus::Run,
            3 => DevStatus::Sleep,
            4 => DevStatus::Privacy,
            b => DevStatus::Unknown(b),
        }
    }
}

/// Decoded fields of the 60-byte selector-0x6 status block (decode shows
/// the byte offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraStatus {
    /// Bytes 0x18/0x1c: current AI tracking mode. None for codes outside
    /// the known table, e.g. (6, 0) reported while switching modes.
    pub ai_mode: Option<AIMode>,
    pub ai_code: (u8, u8),
    pub hdr_on: bool,
    pub dev_status: DevStatus,
    pub sleeping: bool,
    /// Bytes 0x04-0x05: zoom as a percent of the 1.0-2.0x range (0/5/15%
    /// for the 86/78/65 deg FOV modes).
    pub zoom_pct: u16,
    pub face_ae: bool,
    /// Byte 0x11: current field of view; None for value 3 (custom zoom)
    /// and unknown values.
    pub fov: Option<FOVMode>,
    pub mirrored: bool,
    /// Bytes 0x0a-0x0b: auto-sleep delay in seconds; 0 = never.
    pub auto_sleep_secs: i16,
    /// Byte 0x21: LED brightness, 0 = off. Separate from the blink mode
    /// set by Camera::set_led_blinking.
    pub led_brightness: u8,
    /// Byte 0x24: tracking responsiveness, 0 = standard, 2 = sport.
    pub tracker_speed: u8,
    pub fps: u8,
}

impl CameraStatus {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 0x25 {
            return Err(Error::ShortStatus(bytes.len()));
        }
        let m = bytes[0x18];
        let n = bytes[0x1c];
        let dev_status = DevStatus::from_byte(bytes[0x9]);

        Ok(CameraStatus {
            ai_mode: AIMode::from_code(m, n),
            ai_code: (m, n),
            hdr_on: bytes[0x6] != 0,
            dev_status,
            sleeping: dev_status == DevStatus::Sleep,
            zoom_pct: u16::from_le_bytes([bytes[0x4], bytes[0x5]]),
            face_ae: bytes[0x7] != 0,
            fov: FOVMode::from_status(bytes[0x11]),
            mirrored: bytes[0x13] != 0,
            auto_sleep_secs: i16::from_le_bytes([bytes[0xa], bytes[0xb]]),
            led_brightness: bytes[0x21],
            tracker_speed: bytes[0x24],
            fps: bytes[0x1f],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIMode {
    NoTracking,
    NormalTracking,
    UpperBody,
    CloseUp,
    Headless,
    LowerBody,
    DeskMode,
    Whiteboard,
    Hand,
    Group,
}

impl AIMode {
    pub const ALL: [AIMode; 10] = [
        AIMode::NoTracking,
        AIMode::NormalTracking,
        AIMode::UpperBody,
        AIMode::CloseUp,
        AIMode::Headless,
        AIMode::LowerBody,
        AIMode::DeskMode,
        AIMode::Whiteboard,
        AIMode::Hand,
        AIMode::Group,
    ];

    /// Wire (mode, submode) pair, used in the set command and at status
    /// bytes 0x18/0x1c. Mode 6 is a transient the firmware reports while
    /// switching and is never sent.
    pub const fn code(self) -> (u8, u8) {
        match self {
            AIMode::NoTracking => (0, 0),
            AIMode::NormalTracking => (2, 0),
            AIMode::UpperBody => (2, 1),
            AIMode::CloseUp => (2, 2),
            AIMode::Headless => (2, 3),
            AIMode::LowerBody => (2, 4),
            AIMode::DeskMode => (5, 0),
            AIMode::Whiteboard => (4, 0),
            AIMode::Hand => (3, 0),
            AIMode::Group => (1, 0),
        }
    }

    pub fn from_code(mode: u8, submode: u8) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.code() == (mode, submode))
    }

    pub const fn name(self) -> &'static str {
        match self {
            AIMode::NoTracking => "No Tracking",
            AIMode::NormalTracking => "Normal Tracking",
            AIMode::UpperBody => "Upper Body",
            AIMode::CloseUp => "Close-up",
            AIMode::Headless => "Headless",
            AIMode::LowerBody => "Lower Body",
            AIMode::DeskMode => "Desk Mode",
            AIMode::Whiteboard => "Whiteboard",
            AIMode::Hand => "Hand",
            AIMode::Group => "Group",
        }
    }
}

impl Display for AIMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl TryFrom<i32> for AIMode {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AIMode::NoTracking),
            1 => Ok(AIMode::NormalTracking),
            2 => Ok(AIMode::UpperBody),
            3 => Ok(AIMode::CloseUp),
            4 => Ok(AIMode::Headless),
            5 => Ok(AIMode::LowerBody),
            6 => Ok(AIMode::DeskMode),
            7 => Ok(AIMode::Whiteboard),
            8 => Ok(AIMode::Hand),
            9 => Ok(AIMode::Group),
            _ => Err(Error::UnsupportedIntValue("AIMode".to_string(), value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureMode {
    Manual,
    Global,
    Face,
}

impl TryFrom<i32> for ExposureMode {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ExposureMode::Manual),
            1 => Ok(ExposureMode::Global),
            2 => Ok(ExposureMode::Face),
            _ => Err(Error::UnsupportedIntValue(
                "ExposureMode".to_string(),
                value,
            )),
        }
    }
}

pub enum TrackingMode {
    Headroom,
    Standard,
    Motion,
}

impl TryFrom<i32> for TrackingMode {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(TrackingMode::Headroom),
            1 => Ok(TrackingMode::Standard),
            2 => Ok(TrackingMode::Motion),
            _ => Err(Error::UnsupportedIntValue(
                "TrackingMode".to_string(),
                value,
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FOVMode {
    Wide,   // 86°
    Normal, // 78°
    Narrow, // 65°
}

impl FOVMode {
    /// 0-based wire value; the camera ignores 3.
    pub(crate) const fn to_cmd_value(self) -> u8 {
        match self {
            FOVMode::Wide => 0,
            FOVMode::Normal => 1,
            FOVMode::Narrow => 2,
        }
    }

    pub fn from_status(value: u8) -> Option<Self> {
        match value {
            0 => Some(FOVMode::Wide),
            1 => Some(FOVMode::Normal),
            2 => Some(FOVMode::Narrow),
            _ => None,
        }
    }

    pub const fn degrees(self) -> u8 {
        match self {
            FOVMode::Wide => 86,
            FOVMode::Normal => 78,
            FOVMode::Narrow => 65,
        }
    }
}

impl TryFrom<i32> for FOVMode {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FOVMode::Wide),
            1 => Ok(FOVMode::Normal),
            2 => Ok(FOVMode::Narrow),
            _ => Err(Error::UnsupportedIntValue("FOVMode".to_string(), value)),
        }
    }
}

pub trait OBSBotWebCam {
    fn set_ai_mode(&self, mode: AIMode) -> Result<(), Error>;
    fn get_ai_mode(&self) -> Result<AIMode, Error>;
    fn set_hdr_mode(&self, mode: bool) -> Result<(), Error>;
    fn set_exposure_mode(&self, mode: ExposureMode) -> Result<(), Error>;
    fn set_fov(&self, mode: FOVMode) -> Result<(), Error>;
}

impl OBSBotWebCam for Camera {
    fn set_fov(&self, mode: FOVMode) -> Result<(), Error> {
        self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_FOV, 0x01, mode.to_cmd_value()])
    }

    fn set_ai_mode(&self, mode: AIMode) -> Result<(), Error> {
        let (m, n) = mode.code();
        self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_AI_MODE, 0x02, m, n])
    }

    fn set_exposure_mode(&self, mode: ExposureMode) -> Result<(), Error> {
        // Payloads match the frames this library previously replayed as
        // opaque blobs: manual = 5.0 ms shutter, auto = AE level 4.
        match mode {
            // manual = 5.0 ms shutter, auto = AE level 4
            ExposureMode::Manual => {
                self.set_exposure(1, 0x32)?;
            }
            ExposureMode::Global => {
                self.set_exposure(2, 0x04)?;
                self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_FACE_AE, 0x01, 0x00])?;
            }
            ExposureMode::Face => {
                self.set_exposure(2, 0x04)?;
                self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_FACE_AE, 0x01, 0x01])?;
            }
        };
        Ok(())
    }

    fn set_hdr_mode(&self, mode: bool) -> Result<(), Error> {
        self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_HDR, 0x01, mode as u8])
    }

    fn get_ai_mode(&self) -> Result<AIMode, Error> {
        let status = self.get_status()?;
        status
            .ai_mode
            .ok_or(Error::UnknownAIMode(status.ai_code.0, status.ai_code.1))
    }
}

/// Description of one V4L2 control, from VIDIOC_QUERYCTRL.
#[derive(Debug, Clone)]
pub struct CtrlInfo {
    pub id: u32,
    pub name: String,
    /// V4L2_CTRL_TYPE_*: 1=int, 2=bool, 3=menu, 4=button, 6=class, 9=int menu.
    pub ctrl_type: u32,
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
    pub flags: u32,
}

impl From<usbio::v4l2_queryctrl> for CtrlInfo {
    fn from(q: usbio::v4l2_queryctrl) -> Self {
        CtrlInfo {
            id: q.id,
            name: q.name_string(),
            ctrl_type: q.ctrl_type,
            minimum: q.minimum,
            maximum: q.maximum,
            step: q.step,
            default_value: q.default_value,
            flags: q.flags,
        }
    }
}

/// Range information for a V4L2 control (min, max, step, default).
#[derive(Debug, Clone, Copy)]
pub struct CtrlRange {
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
}

impl From<usbio::V4l2CtrlRange> for CtrlRange {
    fn from(r: usbio::V4l2CtrlRange) -> Self {
        CtrlRange {
            minimum: r.minimum,
            maximum: r.maximum,
            step: r.step,
            default_value: r.default_value,
        }
    }
}

impl Camera {
    pub fn new(hint: &str) -> Result<Self, Error> {
        Ok(Self {
            handle: usbio::open_camera(hint)?,
            verbose: false,
            seq: AtomicU16::new(1),
        })
    }

    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// Try to open the camera, retrying every `interval` until it appears.
    /// Prints a message to stderr on the first failure, then silently retries.
    pub fn wait_for(hint: &str, interval: Duration) -> Self {
        let mut warned = false;
        loop {
            match Self::new(hint) {
                Ok(cam) => {
                    if warned {
                        eprintln!("Camera \"{}\" found.", hint);
                    }
                    return cam;
                }
                Err(_) if !warned => {
                    eprintln!("Camera \"{}\" not found, waiting for it to appear...", hint);
                    warned = true;
                }
                Err(_) => {}
            }
            thread::sleep(interval);
        }
    }

    pub fn device_info(&self) -> Result<DeviceInfo, Error> {
        Ok(self.handle.device_info()?)
    }

    pub fn get_status(&self) -> Result<CameraStatus, Error> {
        let mut data: [u8; 60] = [0u8; 60];
        self.get_cur(XU_UNIT_ID, XU_SEL_COMMAND, &mut data)?;
        CameraStatus::decode(&data)
    }

    pub fn read_status_raw(&self) -> Result<[u8; 60], Error> {
        let mut data = [0u8; 60];
        self.get_cur(XU_UNIT_ID, XU_SEL_COMMAND, &mut data)?;
        Ok(data)
    }

    pub fn read_exposure_raw(&self) -> Result<[u8; 60], Error> {
        let mut data = [0u8; 60];
        self.get_cur(XU_UNIT_ID, XU_SEL_EXPOSURE, &mut data)?;
        Ok(data)
    }

    pub fn dump(&self) -> Result<(), Error> {
        hexdump::hexdump(&self.read_status_raw()?);
        Ok(())
    }

    pub fn dump_02(&self) -> Result<(), Error> {
        hexdump::hexdump(&self.read_exposure_raw()?);
        Ok(())
    }

    // ---- Standard V4L2 Pan/Tilt/Zoom controls ----

    /// Get the current absolute pan value (in arc-seconds).
    pub fn get_pan(&self) -> Result<i32, Error> {
        Ok(self.handle.get_ctrl(usbio::V4L2_CID_PAN_ABSOLUTE)?)
    }

    /// Set the absolute pan value (in arc-seconds).
    pub fn set_pan(&self, value: i32) -> Result<(), Error> {
        Ok(self.handle.set_ctrl(usbio::V4L2_CID_PAN_ABSOLUTE, value)?)
    }

    /// Get the current absolute tilt value (in arc-seconds).
    pub fn get_tilt(&self) -> Result<i32, Error> {
        Ok(self.handle.get_ctrl(usbio::V4L2_CID_TILT_ABSOLUTE)?)
    }

    /// Set the absolute tilt value (in arc-seconds).
    pub fn set_tilt(&self, value: i32) -> Result<(), Error> {
        Ok(self.handle.set_ctrl(usbio::V4L2_CID_TILT_ABSOLUTE, value)?)
    }

    /// Get the current absolute zoom value.
    pub fn get_zoom(&self) -> Result<i32, Error> {
        Ok(self.handle.get_ctrl(usbio::V4L2_CID_ZOOM_ABSOLUTE)?)
    }

    /// Set the absolute zoom value.
    pub fn set_zoom(&self, value: i32) -> Result<(), Error> {
        Ok(self.handle.set_ctrl(usbio::V4L2_CID_ZOOM_ABSOLUTE, value)?)
    }

    /// Move pan by a relative amount, clamped to the control's range. The
    /// camera has no relative pan/tilt V4L2 controls, so this is a
    /// read-modify-write of the absolute control; the read comes from the
    /// driver's cache and is stale after tracking or manual moves.
    pub fn pan_relative(&self, delta: i32) -> Result<(), Error> {
        let r = self.query_pan_range()?;
        let v = self.get_pan()?.saturating_add(delta).clamp(r.minimum, r.maximum);
        self.set_pan(v)
    }

    /// Move tilt by a relative amount, clamped (see pan_relative).
    pub fn tilt_relative(&self, delta: i32) -> Result<(), Error> {
        let r = self.query_tilt_range()?;
        let v = self.get_tilt()?.saturating_add(delta).clamp(r.minimum, r.maximum);
        self.set_tilt(v)
    }

    /// Move zoom by a relative amount, clamped (see pan_relative).
    pub fn zoom_relative(&self, delta: i32) -> Result<(), Error> {
        let r = self.query_zoom_range()?;
        let v = self.get_zoom()?.saturating_add(delta).clamp(r.minimum, r.maximum);
        self.set_zoom(v)
    }

    /// Continuous pan at `speed` (sign = direction, 0 stops). Use
    /// query_pan_speed_range for the supported range.
    pub fn set_pan_speed(&self, speed: i32) -> Result<(), Error> {
        Ok(self.handle.set_ctrl(usbio::V4L2_CID_PAN_SPEED, speed)?)
    }

    /// Continuous tilt at `speed` (sign = direction, 0 stops).
    pub fn set_tilt_speed(&self, speed: i32) -> Result<(), Error> {
        Ok(self.handle.set_ctrl(usbio::V4L2_CID_TILT_SPEED, speed)?)
    }

    pub fn query_pan_speed_range(&self) -> Result<CtrlRange, Error> {
        Ok(self.handle.query_ctrl(usbio::V4L2_CID_PAN_SPEED)?.into())
    }

    pub fn query_tilt_speed_range(&self) -> Result<CtrlRange, Error> {
        Ok(self.handle.query_ctrl(usbio::V4L2_CID_TILT_SPEED)?.into())
    }

    /// Query the supported range for pan (absolute).
    pub fn query_pan_range(&self) -> Result<CtrlRange, Error> {
        Ok(self.handle.query_ctrl(usbio::V4L2_CID_PAN_ABSOLUTE)?.into())
    }

    /// Query the supported range for tilt (absolute).
    pub fn query_tilt_range(&self) -> Result<CtrlRange, Error> {
        Ok(self
            .handle
            .query_ctrl(usbio::V4L2_CID_TILT_ABSOLUTE)?
            .into())
    }

    /// Query the supported range for zoom (absolute).
    pub fn query_zoom_range(&self) -> Result<CtrlRange, Error> {
        Ok(self
            .handle
            .query_ctrl(usbio::V4L2_CID_ZOOM_ABSOLUTE)?
            .into())
    }

    pub fn get_ctrl(&self, id: u32) -> Result<i32, Error> {
        Ok(self.handle.get_ctrl(id)?)
    }

    /// The control after `prev_id` (pass 0 to start), None when done.
    pub fn next_ctrl(&self, prev_id: u32) -> Result<Option<CtrlInfo>, Error> {
        Ok(self.handle.next_ctrl(prev_id)?.map(CtrlInfo::from))
    }

    /// Name of one menu entry; None if the index has no entry.
    pub fn query_menu(&self, id: u32, index: u32) -> Result<Option<String>, Error> {
        Ok(self.handle.query_menu(id, index)?)
    }

    pub fn set_ctrl(&self, id: u32, value: i32) -> Result<(), Error> {
        Ok(self.handle.set_ctrl(id, value)?)
    }

    fn next_seq(&self) -> u16 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn send_frame(&self, cmd: (u16, u8), payload: &[u8]) -> Result<u16, Error> {
        let seq = self.next_seq();
        let mut f = frame::build(frame::FLAGS_SET, seq, cmd, payload)?;
        self.set_cur(XU_UNIT_ID, XU_SEL_EXPOSURE, &mut f)?;
        Ok(seq)
    }

    /// Send a framed command and wait for the reply matching this command
    /// and sequence number.
    pub(crate) fn framed_transact(
        &self,
        flags: u8,
        cmd: (u16, u8),
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let seq = self.next_seq();
        let mut f = frame::build(flags, seq, cmd, payload)?;
        self.set_cur(XU_UNIT_ID, XU_SEL_EXPOSURE, &mut f)?;

        // The mailbox keeps the previous reply until a new one arrives,
        // so match on command and sequence number.
        for _ in 0..10 {
            thread::sleep(Duration::from_millis(60));
            let mut buf = [0u8; 60];
            self.get_cur(XU_UNIT_ID, XU_SEL_EXPOSURE, &mut buf)?;
            if let Ok(reply) = frame::parse(&buf) {
                if reply.cmd == cmd.0 && reply.seq == seq {
                    return Ok(reply.payload);
                }
            }
        }
        Err(Error::NoFrameReply { cmd: cmd.0, seq })
    }

    /// Wake from auto-sleep; most commands are ignored while asleep.
    pub fn wake(&self) -> Result<(), Error> {
        self.send_frame(frame::cmd::CAM_SET_DEV_STATUS, &0u32.to_le_bytes())?;
        Ok(())
    }

    pub fn sleep(&self) -> Result<(), Error> {
        self.send_frame(frame::cmd::CAM_SET_DEV_STATUS, &1u32.to_le_bytes())?;
        Ok(())
    }

    pub fn wake_and_wait(&self, timeout: Duration) -> Result<(), Error> {
        if !self.get_status()?.sleeping {
            return Ok(());
        }
        self.wake()?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            thread::sleep(Duration::from_millis(150));
            match self.get_status() {
                Ok(s) if !s.sleeping => {
                    // The firmware still drops some commands for a while
                    // after the sleep flag clears.
                    thread::sleep(Duration::from_millis(400));
                    return Ok(());
                }
                _ if std::time::Instant::now() >= deadline => {
                    return Err(Error::NoFrameReply {
                        cmd: frame::cmd::CAM_SET_DEV_STATUS.0,
                        seq: 0,
                    })
                }
                _ => {}
            }
        }
    }

    pub fn set_mirror(&self, on: bool) -> Result<(), Error> {
        self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_MIRROR, 0x01, on as u8])
    }

    /// Selects blinking vs solid (not on/off). Write-only; brightness is a
    /// separate setting (CameraStatus::led_brightness).
    pub fn set_led_blinking(&self, blinking: bool) -> Result<(), Error> {
        self.send_cmd(
            XU_UNIT_ID,
            XU_SEL_COMMAND,
            &[TAG_LED, 0x01, blinking as u8],
        )
    }

    /// Drive the gimbal at degrees per second until gimbal_stop (or zero
    /// speeds). Positive yaw pans right, positive pitch tilts down.
    pub fn gimbal_speed(&self, yaw_dps: f32, pitch_dps: f32) -> Result<(), Error> {
        let mut p = [0u8; 12];
        p[4..8].copy_from_slice(&pitch_dps.to_le_bytes());
        p[8..12].copy_from_slice(&yaw_dps.to_le_bytes());
        self.send_frame(frame::cmd::AI_SET_GIM_SPEED, &p)?;
        Ok(())
    }

    pub fn gimbal_stop(&self) -> Result<(), Error> {
        self.gimbal_speed(0.0, 0.0)
    }

    /// Absolute move in degrees (same sign convention as gimbal_speed).
    pub fn gimbal_move_to(&self, yaw_deg: f32, pitch_deg: f32) -> Result<(), Error> {
        let mut p = [0u8; 12];
        p[4..8].copy_from_slice(&pitch_deg.to_le_bytes());
        p[8..12].copy_from_slice(&yaw_deg.to_le_bytes());
        self.send_frame(frame::cmd::AI_SET_GIM_MOTOR_DEG, &p)?;
        Ok(())
    }

    pub fn gimbal_recenter(&self) -> Result<(), Error> {
        self.send_frame(frame::cmd::GIM_SET_MOTOR, &[0u8; 6])?;
        Ok(())
    }

    pub fn gimbal_state_raw(&self) -> Result<Vec<u8>, Error> {
        self.framed_transact(frame::FLAGS_GET, frame::cmd::AI_GET_GIM_STATE, &[])
    }

    /// `ratio_x100`: 100 = 1.0x, 200 = 2.0x. Speed is clamped to 1-10;
    /// the camera discards larger values.
    pub fn set_zoom_with_speed(&self, ratio_x100: u32, speed: u32) -> Result<(), Error> {
        let speed = speed.clamp(1, 10);
        let mut p = [0u8; 8];
        p[0..4].copy_from_slice(&speed.to_le_bytes());
        p[4..8].copy_from_slice(&ratio_x100.to_le_bytes());
        self.send_frame(frame::cmd::CAM_SET_ZOOM_ABSOLUTE, &p)?;
        Ok(())
    }

    pub fn zoom_stop(&self) -> Result<(), Error> {
        self.send_frame(frame::cmd::CAM_SET_ZOOM_STOP, &[])?;
        Ok(())
    }

    /// false = standard, true = sport; the camera ignores values other
    /// than 0 and 2.
    pub fn set_track_speed_sport(&self, sport: bool) -> Result<(), Error> {
        self.send_frame(
            frame::cmd::AI_SET_TRACK_MODE,
            &[if sport { 2 } else { 0 }],
        )?;
        Ok(())
    }

    pub fn set_suspend_time(&self, secs: u16) -> Result<(), Error> {
        let b = secs.to_le_bytes();
        self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_SUSPEND_TIME, 0x02, b[0], b[1]])
    }

    pub fn set_sleep_without_stream_disabled(&self, disabled: bool) -> Result<(), Error> {
        self.send_cmd(
            XU_UNIT_ID,
            XU_SEL_COMMAND,
            &[TAG_NO_SLEEP_WITHOUT_STREAM, 0x01, disabled as u8],
        )
    }

    pub fn set_mic_during_sleep(&self, on: bool) -> Result<(), Error> {
        self.send_cmd(
            XU_UNIT_ID,
            XU_SEL_COMMAND,
            &[TAG_MIC_DURING_SLEEP, 0x01, on as u8],
        )
    }

    /// Accepted but has no observable effect on the firmware tested.
    pub fn set_vertical_mode(&self, vertical: bool) -> Result<(), Error> {
        self.send_cmd(
            XU_UNIT_ID,
            XU_SEL_COMMAND,
            &[TAG_VERTICAL, 0x01, vertical as u8],
        )
    }

    pub fn set_boot_ai_mode(&self, mode: AIMode) -> Result<(), Error> {
        let (m, n) = mode.code();
        self.send_cmd(XU_UNIT_ID, XU_SEL_COMMAND, &[TAG_BOOT_MODE, 0x02, m, n])
    }

    /// mode 1 = manual (shutter in 0.1 ms, snapped by the camera),
    /// 2 = auto (AE level).
    pub fn set_exposure(&self, mode: u8, value: u32) -> Result<(), Error> {
        let mut payload = [0u8; 5];
        payload[0] = mode;
        payload[1..5].copy_from_slice(&value.to_le_bytes());
        self.send_frame(frame::cmd::CAM_SET_EXPOSURE, &payload)?;
        Ok(())
    }

    /// mode 1 = manual, 2 = auto.
    pub fn get_exposure(&self) -> Result<(u8, u32), Error> {
        let p = self.framed_transact(frame::FLAGS_GET, frame::cmd::CAM_GET_EXPOSURE, &[])?;
        if p.len() < 5 {
            return Err(Error::BadFrameReply {
                cmd: frame::cmd::CAM_GET_EXPOSURE.0,
                payload: p,
            });
        }
        Ok((p[0], u32::from_le_bytes([p[1], p[2], p[3], p[4]])))
    }

    /// Serial number (not present in the USB descriptors).
    pub fn serial_number(&self) -> Result<String, Error> {
        let p = self.framed_transact(frame::FLAGS_GET, frame::cmd::UG_GET_SN, &[])?;
        if p.is_empty() {
            return Err(Error::BadFrameReply {
                cmd: frame::cmd::UG_GET_SN.0,
                payload: p,
            });
        }
        Ok(String::from_utf8_lossy(&p)
            .trim_end_matches('\0')
            .to_string())
    }

    pub fn get_suspend_time(&self) -> Result<u32, Error> {
        let p = self.framed_transact(frame::FLAGS_GET, frame::cmd::CAM_GET_SUSPEND_TIME, &[])?;
        if p.len() < 4 {
            return Err(Error::BadFrameReply {
                cmd: frame::cmd::CAM_GET_SUSPEND_TIME.0,
                payload: p,
            });
        }
        Ok(u32::from_le_bytes([p[0], p[1], p[2], p[3]]))
    }

    pub fn send_cmd(&self, unit: u8, selector: u8, cmd: &[u8]) -> Result<(), Error> {
        let mut data = [0u8; 60];
        if cmd.len() > data.len() {
            return Err(Error::BufferSize {
                expected: data.len(),
                got: cmd.len(),
            });
        }
        data[..cmd.len()].copy_from_slice(cmd);

        self.set_cur(unit, selector, &mut data)
    }

    pub(crate) fn get_cur_block(
        &self,
        unit: u8,
        selector: u8,
        data: &mut [u8],
    ) -> Result<(), Error> {
        self.get_cur(unit, selector, data)
    }

    pub(crate) fn set_cur_block(
        &self,
        unit: u8,
        selector: u8,
        data: &mut [u8],
    ) -> Result<(), Error> {
        self.set_cur(unit, selector, data)
    }

    fn get_cur(&self, unit: u8, selector: u8, data: &mut [u8]) -> Result<(), Error> {
        // always call get_len first
        let size = self.get_len(unit, selector)?;
        if data.len() < size {
            if self.verbose {
                eprintln!("get_cur: buffer too small, control size {}", size);
            }
            return Err(Error::BufferSize {
                expected: size,
                got: data.len(),
            });
        }

        Ok(self.handle.io(unit, selector, usbio::UVC_GET_CUR, data)?)
    }

    fn set_cur(&self, unit: u8, selector: u8, data: &mut [u8]) -> Result<(), Error> {
        let size = self.get_len(unit, selector)?;
        if data.len() > size {
            if self.verbose {
                eprintln!("set_cur: buffer too large, control size {}", size);
            }
            return Err(Error::BufferSize {
                expected: size,
                got: data.len(),
            });
        }

        if self.verbose {
            eprintln!("set_cur: {} {} {}", unit, selector, hex::encode(&data));
        }

        Ok(self.handle.io(unit, selector, usbio::UVC_SET_CUR, data)?)
    }

    fn get_len(&self, unit: u8, selector: u8) -> Result<usize, Error> {
        let mut data = [0u8; 2];
        self.handle.io(unit, selector, usbio::UVC_GET_LEN, &mut data)?;
        Ok(u16::from_le_bytes(data).into())
    }

    pub(crate) fn handle(&self) -> &usbio::CameraHandle {
        &self.handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_mode_code_roundtrip() {
        for mode in AIMode::ALL {
            let (m, n) = mode.code();
            assert_eq!(AIMode::from_code(m, n), Some(mode));
        }
        assert_eq!(AIMode::from_code(3, 0), Some(AIMode::Hand));
        assert_eq!(AIMode::from_code(6, 0), None); // mode-switch transient
        assert_eq!(AIMode::from_code(2, 5), None);
    }

    #[test]
    fn fov_cmd_status_roundtrip() {
        for (mode, value) in [
            (FOVMode::Wide, 0),
            (FOVMode::Normal, 1),
            (FOVMode::Narrow, 2),
        ] {
            assert_eq!(mode.to_cmd_value(), value);
            assert_eq!(FOVMode::from_status(mode.to_cmd_value()), Some(mode));
        }
        assert_eq!(FOVMode::from_status(3), None);
    }

    #[test]
    fn status_decode() {
        let mut block = [0u8; 60];
        block[0x6] = 1;
        block[0x11] = 2;
        block[0x4] = 15;
        block[0x9] = 3;
        block[0xa] = 120;
        block[0x1f] = 25;
        block[0x21] = 1;
        let s = CameraStatus::decode(&block).unwrap();
        assert!(s.hdr_on);
        assert_eq!(s.ai_mode, Some(AIMode::NoTracking));
        assert_eq!(s.fov, Some(FOVMode::Narrow));
        assert_eq!(s.zoom_pct, 15);
        assert!(s.sleeping);
        assert_eq!(s.dev_status, DevStatus::Sleep);
        assert_eq!(s.auto_sleep_secs, 120);
        assert_eq!(s.fps, 25);
        assert_eq!(s.led_brightness, 1);
        assert!(!s.face_ae);
        block[0x11] = 3;
        assert_eq!(CameraStatus::decode(&block).unwrap().fov, None);
        block[0x11] = 2;

        block[0x18] = 2;
        block[0x1c] = 1;
        let s = CameraStatus::decode(&block).unwrap();
        assert_eq!(s.ai_mode, Some(AIMode::UpperBody));

        block[0x18] = 3;
        block[0x1c] = 0;
        assert_eq!(
            CameraStatus::decode(&block).unwrap().ai_mode,
            Some(AIMode::Hand)
        );

        // (6, 0) is the mode-switch transient; the rest must still decode.
        block[0x18] = 6;
        let s = CameraStatus::decode(&block).unwrap();
        assert_eq!(s.ai_mode, None);
        assert_eq!(s.ai_code, (6, 0));
        assert!(s.hdr_on);

        block[0x18] = 9;
        block[0x1c] = 1;
        assert_eq!(CameraStatus::decode(&block).unwrap().ai_mode, None);

        assert!(matches!(
            CameraStatus::decode(&[0u8; 10]),
            Err(Error::ShortStatus(10))
        ));
    }
}
