// SPDX-License-Identifier: EUPL-1.2

use enum_dispatch::enum_dispatch;
use errno::Errno;
use nix::errno::errno;
use nix::{ioctl_read_buf, ioctl_readwrite, ioctl_readwrite_buf};
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
    fn get_ctrl(&self, id: u32) -> Result<i32, Errno>;
    fn set_ctrl(&self, id: u32, value: i32) -> Result<(), Errno>;
    fn query_ctrl(&self, id: u32) -> Result<V4l2CtrlRange, Errno>;
}

/// Range information for a V4L2 control, returned by VIDIOC_QUERYCTRL.
#[derive(Debug, Clone, Copy)]
pub struct V4l2CtrlRange {
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
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

    fn get_ctrl(&self, id: u32) -> Result<i32, Errno> {
        let dev = &self.0;
        let mut ctrl = v4l2_control { id, value: 0 };

        unsafe {
            match vidioc_g_ctrl(dev.as_raw_fd(), &mut ctrl) {
                Ok(_) => Ok(ctrl.value),
                _ => Err(errno::Errno(errno())),
            }
        }
    }

    fn set_ctrl(&self, id: u32, value: i32) -> Result<(), Errno> {
        let dev = &self.0;
        let mut ctrl = v4l2_control { id, value };

        unsafe {
            match vidioc_s_ctrl(dev.as_raw_fd(), &mut ctrl) {
                Ok(_) => Ok(()),
                _ => Err(errno::Errno(errno())),
            }
        }
    }

    fn query_ctrl(&self, id: u32) -> Result<V4l2CtrlRange, Errno> {
        let dev = &self.0;
        let mut qctrl = v4l2_queryctrl {
            id,
            ..Default::default()
        };

        unsafe {
            match vidioc_queryctrl(dev.as_raw_fd(), &mut qctrl) {
                Ok(_) => {
                    if qctrl.flags & V4L2_CTRL_FLAG_DISABLED != 0 {
                        return Err(errno::Errno(22)); // EINVAL
                    }
                    Ok(V4l2CtrlRange {
                        minimum: qctrl.minimum,
                        maximum: qctrl.maximum,
                        step: qctrl.step,
                        default_value: qctrl.default_value,
                    })
                }
                _ => Err(errno::Errno(errno())),
            }
        }
    }
}

pub(crate) fn open_camera(hint: &str) -> Result<CameraHandle, crate::Error> {
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

// ---- Standard V4L2 controls for Pan/Tilt/Zoom ----

/// V4L2 control struct for VIDIOC_G_CTRL / VIDIOC_S_CTRL
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Default, Debug)]
pub struct v4l2_control {
    pub id: u32,
    pub value: i32,
}

/// V4L2 queryctrl struct for VIDIOC_QUERYCTRL
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Default, Debug)]
pub struct v4l2_queryctrl {
    pub id: u32,
    pub ctrl_type: u32,
    pub name: [u8; 32],
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
    pub flags: u32,
    pub reserved: [u32; 2],
}

const V4L2_CTRL_FLAG_DISABLED: u32 = 0x0001;

// VIDIOC_G_CTRL = _IOWR('V', 27, struct v4l2_control)
// VIDIOC_S_CTRL = _IOWR('V', 28, struct v4l2_control)
// VIDIOC_QUERYCTRL = _IOWR('V', 36, struct v4l2_queryctrl)
ioctl_readwrite!(vidioc_g_ctrl, b'V', 27, v4l2_control);
ioctl_readwrite!(vidioc_s_ctrl, b'V', 28, v4l2_control);
ioctl_readwrite!(vidioc_queryctrl, b'V', 36, v4l2_queryctrl);

// Standard V4L2 Camera Control IDs
// V4L2_CID_CAMERA_CLASS_BASE = 0x009A0900
#[allow(dead_code)]
pub const V4L2_CID_PAN_ABSOLUTE: u32 = 0x009A0908;
#[allow(dead_code)]
pub const V4L2_CID_TILT_ABSOLUTE: u32 = 0x009A0909;
#[allow(dead_code)]
pub const V4L2_CID_PAN_RELATIVE: u32 = 0x009A090A;
#[allow(dead_code)]
pub const V4L2_CID_TILT_RELATIVE: u32 = 0x009A090B;
#[allow(dead_code)]
pub const V4L2_CID_ZOOM_ABSOLUTE: u32 = 0x009A090D;
#[allow(dead_code)]
pub const V4L2_CID_ZOOM_RELATIVE: u32 = 0x009A090E;
