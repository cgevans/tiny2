// SPDX-License-Identifier: EUPL-1.2

use enum_dispatch::enum_dispatch;
use errno::Errno;
use nix::errno::errno;
use nix::{ioctl_read_buf, ioctl_readwrite_buf};
use std::fs::File;
use std::os::unix::io::AsRawFd;
//use std::fs::OpenOptions;
//use std::os::unix::fs::OpenOptionsExt;
use glob::glob_with;
use glob::MatchOptions;
use std::str;

#[enum_dispatch(CameraHandleType)]
pub trait UvcUsbIo {
    fn info(&self) -> Result<(), Errno>;
    fn io(&self, unit: u8, selector: u8, query: u8, data: &mut [u8]) -> Result<(), Errno>;
}

#[derive(Debug)]
pub struct CameraHandle(std::fs::File);

impl From<File> for CameraHandle {
    fn from(file: File) -> Self {
        CameraHandle(file)
    }
}

impl UvcUsbIo for CameraHandle {
    fn info(&self) -> Result<(), Errno> {
        match v4l2_capability::new(&self.0) {
            Ok(video_info) => {
                println!(
                    "Card: {}\nBus : {}",
                    str::from_utf8(&video_info.card).unwrap(),
                    str::from_utf8(&video_info.bus_info).unwrap()
                );
                Ok(())
            }
            _ => {
                println!("Failed");
                Err(errno::Errno(errno()))
            }
        }
    }

    fn io(&self, unit: u8, selector: u8, query: u8, data: &mut [u8]) -> Result<(), Errno> {
        let dev = &self.0;

        let query = uvc_xu_control_query {
            unit,
            selector,
            query,
            size: data.len() as u16,
            data: data.as_mut_ptr(),
        };

        unsafe {
            match uvcioc_ctrl_query(dev.as_raw_fd(), &mut [query]) {
                Ok(_) => Ok(()),
                _ => Err(errno::Errno(errno())),
            }
        }
    }
}

pub(crate)
fn open_camera(hint: &str) -> Result<CameraHandle, crate::Error> {
    if let Ok(file) = File::open(hint) {
        return Ok(file.into());
    }

    if let Ok(file) = File::open("/dev/".to_owned() + hint) {
        return Ok(file.into());
    }

    // enumerate all cameras and check for match
    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    for path in glob_with("/dev/video*", options).unwrap().flatten() {
        if let Ok(device) = File::open(&path) {
            if let Ok(video_info) = v4l2_capability::new(&device) {
                // println!("Info: {}\nCard: {:?}\nBus:  {:?}\ndc {:#X}", , video_info.card, video_info.bus_info, video_info.device_caps & 0x800000);
                if (str::from_utf8(&video_info.card).unwrap().contains(hint)
                    || str::from_utf8(&video_info.bus_info).unwrap().contains(hint))
                    && (video_info.device_caps & 0x800000 == 0)
                {
                    return Ok(device.into());
                }
            }
        }
    }
    Err(crate::Error::NoCameraFound)
}

#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Default, Debug)]
pub struct v4l2_capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

impl v4l2_capability {
    fn new(dev: &std::fs::File) -> Result<Self, errno::Errno> {
        let mut query = [v4l2_capability {
            ..Default::default()
        }];

        unsafe {
            match ioctl_videoc_querycap(dev.as_raw_fd(), &mut query) {
                Ok(_) => Ok(query[0]),
                _ => Err(errno::Errno(errno())),
            }
        }
    }
}

const VIDIOC_QUERYCAP_MAGIC: u8 = b'V';
const VIDIOC_QUERYCAP_QUERY_MESSAGE: u8 = 0x00;
ioctl_read_buf!(
    ioctl_videoc_querycap,
    VIDIOC_QUERYCAP_MAGIC,
    VIDIOC_QUERYCAP_QUERY_MESSAGE,
    v4l2_capability
);

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct uvc_xu_control_query {
    unit: u8,
    selector: u8,
    query: u8, /* Video Class-Specific Request Code, */
    /* defined in linux/usb/video.h A.8.  */
    size: u16,
    data: *mut u8,
}

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct uvc_menu_info {
    name: [u8; 32],
    value: u32,
}

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct uvc_xu_control_mapping {
    id: u32,
    name: [u8; 32],
    entity: u8,
    selector: u8,
    size: u16,
    offset: u16,
    v4l2_type: u32,
    data_type: u32,
    uvc_menu_info: u32,
    uvc_menu_count: u32,
}

const UVCIOC_CTRL_MAGIC: u8 = b'u'; // Defined in linux/uvcvideo.h
const UVCIOC_CTRL_QUERY_MESSAGE: u8 = 0x21; // Defined in linux/uvcvideo.h
ioctl_readwrite_buf!(
    uvcioc_ctrl_query,
    UVCIOC_CTRL_MAGIC,
    UVCIOC_CTRL_QUERY_MESSAGE,
    uvc_xu_control_query
);
ioctl_read_buf!(
    uvcioc_ctrl_query_read,
    UVCIOC_CTRL_MAGIC,
    UVCIOC_CTRL_QUERY_MESSAGE,
    uvc_xu_control_query
);

/* A.8. Video Class-Specific Request Codes */
#[allow(dead_code)]
const UVC_RC_UNDEFINED: u8 = 0x00;
#[allow(dead_code)]
pub const UVC_SET_CUR: u8 = 0x01;
#[allow(dead_code)]
pub const UVC_GET_CUR: u8 = 0x81;
#[allow(dead_code)]
const UVC_GET_MIN: u8 = 0x82;
#[allow(dead_code)]
const UVC_GET_MAX: u8 = 0x83;
#[allow(dead_code)]
const UVC_GET_RES: u8 = 0x84;
#[allow(dead_code)]
pub const UVC_GET_LEN: u8 = 0x85;
#[allow(dead_code)]
const UVC_GET_INFO: u8 = 0x86;
#[allow(dead_code)]
const UVC_GET_DEF: u8 = 0x87;
