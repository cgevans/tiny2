// SPDX-License-Identifier: EUPL-1.2

mod usbio;

use errno::Errno;
use usbio::UvcUsbIo;
use std::{fmt::Display, io};
use thiserror::Error;

const AUTO_EXP_CMD: [u8; 18] = [0xaa, 0x25, 0x16, 0x00, 0x0c, 0x00, 0x58, 0x91, 0x0a, 0x02, 0x82, 0x29, 0x05, 0x00, 0xb2, 0xaf, 0x02, 0x04];
const MANUAL_EXP_CMD: [u8; 18] = [0xaa, 0x25, 0x15, 0x00, 0x0c, 0x00, 0xa8, 0x9e, 0x0a, 0x02, 0x82, 0x29, 0x05, 0x00, 0xf9, 0x27, 0x01, 0x32];

#[derive(Error, Debug)]
pub enum Error {
    #[error("value of {1} is not supported for {0}")]
    UnsupportedIntValue(String, i32),
    #[error("USB IO error: {0}")]
    USBIOError(i32),
    #[error("IO error: {0}")]
    IOError(#[from] io::Error),
    #[error("Osc error: {0}")]
    OscError(#[from] rosc::OscError),
    #[error("no camera found")]
    NoCameraFound,
}

#[derive(Debug)]
pub struct Camera {
    handle: usbio::CameraHandle,
}

pub struct CameraStatus {
    pub ai_mode: AIMode,
    pub hdr_on: bool
}

impl CameraStatus {
    pub fn decode(bytes: &[u8]) -> Self {
        let m = bytes[0x18];
        let n = bytes[0x1c];

        let ai_mode = match (m, n) {
            (0, 0) => AIMode::NoTracking,
            (2, 0) => AIMode::NormalTracking,
            (2, 1) => AIMode::UpperBody,
            (2, 2) => AIMode::CloseUp,
            (2, 3) => AIMode::Headless,
            (2, 4) => AIMode::LowerBody,
            (5, 0) => AIMode::DeskMode,
            (4, 0) => AIMode::Whiteboard,
            (6, 0) => AIMode::Hand,
            (1, 0) => AIMode::Group,
            (_, _) => panic!(),
        };

        let hdr_on = bytes[0x6] != 0;

        CameraStatus { ai_mode, hdr_on }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExposureMode {
    Manual,
    Global,
    Face
}

impl Display for AIMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIMode::NoTracking => write!(f, "No Tracking"),
            AIMode::NormalTracking => write!(f, "Normal Tracking"),
            AIMode::UpperBody => write!(f, "Upper Body"),
            AIMode::CloseUp => write!(f, "Close-up"),
            AIMode::Headless => write!(f, "Headless"),
            AIMode::LowerBody => write!(f, "Lower Body"),
            AIMode::DeskMode => write!(f, "Desk Mode"),
            AIMode::Whiteboard => write!(f, "Whiteboard"),
            AIMode::Hand => write!(f, "Hand"),
            AIMode::Group => write!(f, "Group"),
        }
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

pub trait OBSBotWebCam {
    fn set_ai_mode(&self, mode: AIMode) -> Result<(), Error>;
    fn get_ai_mode(&self) -> Result<AIMode, Error>;
    fn set_hdr_mode(&self, mode: bool) -> Result<(), Error>;
    fn set_exposure_mode(&self, mode: ExposureMode) -> Result<(), Error>;
}

impl OBSBotWebCam for Camera {
    fn set_ai_mode(&self, mode: AIMode) -> Result<(), Error> {
        let cmd = match mode {
            AIMode::NoTracking => [0x16, 0x02, 0x00, 0x00],
            AIMode::NormalTracking => [0x16, 0x02, 0x02, 0x00],
            AIMode::UpperBody => [0x16, 0x02, 0x02, 0x01],
            AIMode::DeskMode => [0x16, 0x02, 0x05, 0x00],
            AIMode::Whiteboard => [0x16, 0x02, 0x04, 0x00],
            AIMode::Group => [0x16, 0x02, 0x01, 0x00],
            AIMode::Hand => [0x16, 0x02, 0x03, 0x00],
            AIMode::CloseUp => [0x16, 0x02, 0x02, 0x02],
            AIMode::Headless => [0x16, 0x02, 0x02, 0x03],
            AIMode::LowerBody => [0x16, 0x02, 0x02, 0x04],
        };
        self.send_cmd(0x2, 0x6, &cmd)
    }

    fn set_exposure_mode(&self, mode: ExposureMode) -> Result<(), Error> {
        match mode {
            ExposureMode::Manual => {
                self.send_cmd(0x2, 0x2, &MANUAL_EXP_CMD)?;
            }
            ExposureMode::Global => {
                self.send_cmd(0x2, 0x6, &AUTO_EXP_CMD)?;
                self.send_cmd(0x2, 0x6, &[0x03, 0x01, 0x00])?;
            }
            ExposureMode::Face => {
                self.send_cmd(0x2, 0x6, &AUTO_EXP_CMD)?;
                self.send_cmd(0x2, 0x6, &[0x03, 0x01, 0x01])?;
            }
        };
        Ok(())
    }



    fn set_hdr_mode(&self, mode: bool) -> Result<(), Error> {
        let cmd = if mode {
            [0x01, 0x01, 0x01]
        } else {
            [0x01, 0x01, 0x00]
        };
        self.send_cmd(0x2, 0x6, &cmd)
    }

    fn get_ai_mode(&self) -> Result<AIMode, Error> {
        Ok(self.get_status()?.ai_mode)
    }
}

impl Camera {
    pub fn new(hint: &str) -> Result<Self, Error> {
        Ok(Self { handle: usbio::open_camera(hint)? })
    }

    pub fn info(&self) -> Result<(), Errno> {
        self.handle.info()
    }

    pub fn get_status(&self) -> Result<CameraStatus, Error> {
        let mut data: [u8; 60] = [0u8; 60];
        self.get_cur(0x2, 0x6, &mut data)
            .map_err(|x| Error::USBIOError(x.0))?;
        Ok(CameraStatus::decode(&data))
    }

    pub fn dump(&self) -> Result<(), Errno> {
        let mut data: [u8; 60] = [0u8; 60];
        self.get_cur(0x2, 0x6, &mut data)?;
        hexdump::hexdump(&data);
        Ok(())
    }

    pub fn dump_02(&self) -> Result<(), Errno> {
        let mut data: [u8; 60] = [0u8; 60];
        self.get_cur(0x2, 0x2, &mut data)?;
        hexdump::hexdump(&data);
        Ok(())
    }

    pub fn send_cmd(&self, unit: u8, selector: u8, cmd: &[u8]) -> Result<(), Error> {
        let mut data = [0u8; 60];
        data[..cmd.len()].copy_from_slice(cmd);

        self.set_cur(unit, selector, &mut data)
            .map_err(|e| Error::USBIOError(e.0))
    }

    fn get_cur(&self, unit: u8, selector: u8, data: &mut [u8]) -> Result<(), errno::Errno> {
        // always call get_len first
        match self.get_len(unit, selector) {
            Ok(size) => {
                if data.len() < size {
                    println!("Got size {}", size);
                    return Err(errno::Errno(1));
                }
            }
            Err(err) => return Err(err),
        };

        // Why not &mut data here?
        match self.io(unit, selector, usbio::UVC_GET_CUR, data) {
            Ok(_) => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn set_cur(&self, unit: u8, selector: u8, data: &mut [u8]) -> Result<(), errno::Errno> {
        match self.get_len(unit, selector) {
            Ok(size) => {
                if data.len() > size {
                    println!("Got size {}", size);
                    return Err(errno::Errno(1));
                }
            }
            Err(err) => return Err(err),
        };

        println!("{:} {:} {:}", unit, selector, hex::encode(&data));

        match self.io(unit, selector, usbio::UVC_SET_CUR, data) {
            Ok(_) => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn get_len(&self, unit: u8, selector: u8) -> Result<usize, Errno> {
        let mut data = [0u8; 2];

        match self.io(unit, selector, usbio::UVC_GET_LEN, &mut data) {
            Ok(_) => Ok(u16::from_le_bytes(data).into()),
            Err(err) => Err(err),
        }
    }

    fn io(&self, unit: u8, selector: u8, query: u8, data: &mut [u8]) -> Result<(), Errno> {
        self.handle.io(unit, selector, query, data)
    }
}
