// SPDX-License-Identifier: EUPL-1.2

//! Low-level V4L2 / UVC ioctl access, mirroring the kernel UAPI in
//! linux/videodev2.h and linux/uvcvideo.h. The layout and ioctl-number
//! notes are LLM-drafted (Claude); the `abi` test module checks the struct
//! layouts against the C headers.

use glob::{glob_with, MatchOptions};
use nix::errno::Errno;
use nix::{ioctl_read_buf, ioctl_readwrite, ioctl_readwrite_buf};
use std::fs::File;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;

#[derive(Debug)]
pub struct CameraHandle(std::fs::File);

impl From<File> for CameraHandle {
    fn from(file: File) -> Self {
        CameraHandle(file)
    }
}

/// Range information for a V4L2 control, returned by VIDIOC_QUERYCTRL.
#[derive(Debug, Clone, Copy)]
pub struct V4l2CtrlRange {
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
}

pub struct DeviceInfo {
    pub card: String,
    pub bus_info: String,
}

/// A dynamic UVC-to-V4L2 control mapping request (UVCIOC_CTRL_MAP);
/// map_ctrl translates it into the kernel's uvc_xu_control_mapping.
#[derive(Debug, Clone)]
pub struct XuCtrlMap<'a> {
    pub id: u32,
    pub name: &'a str,
    pub entity: [u8; 16],
    pub selector: u8,
    pub size_bits: u8,
    pub offset_bits: u8,
    pub v4l2_type: u32,
    pub data_type: u32,
    pub menu: &'a [(&'a str, u32)],
}

pub const V4L2_CTRL_TYPE_MENU: u32 = 3;
pub const UVC_CTRL_DATA_TYPE_ENUM: u32 = 4;

impl CameraHandle {
    pub fn device_info(&self) -> Result<DeviceInfo, Errno> {
        let caps = v4l2_capability::new(&self.0)?;
        Ok(DeviceInfo {
            card: nul_terminated(&caps.card),
            bus_info: nul_terminated(&caps.bus_info),
        })
    }

    pub fn io(&self, unit: u8, selector: u8, query: u8, data: &mut [u8]) -> Result<(), Errno> {
        let query = uvc_xu_control_query {
            unit,
            selector,
            query,
            size: data.len() as u16,
            data: data.as_mut_ptr(),
        };

        unsafe { uvcioc_ctrl_query(self.0.as_raw_fd(), &mut [query]) }.map(drop)
    }

    pub fn get_ctrl(&self, id: u32) -> Result<i32, Errno> {
        let mut ctrl = v4l2_control { id, value: 0 };
        unsafe { vidioc_g_ctrl(self.0.as_raw_fd(), &mut ctrl) }?;
        Ok(ctrl.value)
    }

    pub fn set_ctrl(&self, id: u32, value: i32) -> Result<(), Errno> {
        let mut ctrl = v4l2_control { id, value };
        unsafe { vidioc_s_ctrl(self.0.as_raw_fd(), &mut ctrl) }.map(drop)
    }

    pub fn query_ctrl(&self, id: u32) -> Result<V4l2CtrlRange, Errno> {
        let qctrl = self.query_ctrl_raw(id)?;
        if qctrl.flags & V4L2_CTRL_FLAG_DISABLED != 0 {
            return Err(Errno::EINVAL);
        }
        Ok(V4l2CtrlRange {
            minimum: qctrl.minimum,
            maximum: qctrl.maximum,
            step: qctrl.step,
            default_value: qctrl.default_value,
        })
    }

    pub fn query_ctrl_raw(&self, id: u32) -> Result<v4l2_queryctrl, Errno> {
        let mut qctrl = v4l2_queryctrl {
            id,
            ..Default::default()
        };
        unsafe { vidioc_queryctrl(self.0.as_raw_fd(), &mut qctrl) }?;
        Ok(qctrl)
    }

    /// The control after `prev_id` in the device's control list (pass 0 to
    /// start); None when the list is exhausted.
    pub fn next_ctrl(&self, prev_id: u32) -> Result<Option<v4l2_queryctrl>, Errno> {
        match self.query_ctrl_raw(prev_id | V4L2_CTRL_FLAG_NEXT_CTRL) {
            Ok(q) => Ok(Some(q)),
            Err(Errno::EINVAL) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Name of one menu entry; None if the index has no entry.
    pub fn query_menu(&self, id: u32, index: u32) -> Result<Option<String>, Errno> {
        let mut qm = v4l2_querymenu {
            id,
            index,
            name: [0; 32],
            reserved: 0,
        };
        match unsafe { vidioc_querymenu(self.0.as_raw_fd(), &mut qm) } {
            Ok(_) => Ok(Some(nul_terminated(&{ qm.name }))),
            // EINVAL: no entry at this index
            Err(Errno::EINVAL) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Install a dynamic control mapping (UVCIOC_CTRL_MAP). Returns
    /// Ok(true) if installed, Ok(false) if the id was already mapped
    /// (EEXIST); the existing mapping stays in effect until the device is
    /// unplugged.
    pub fn map_ctrl(&self, map: &XuCtrlMap<'_>) -> Result<bool, Errno> {
        let mut menu: Vec<uvc_menu_info> = map
            .menu
            .iter()
            .map(|(name, value)| {
                let mut mi = uvc_menu_info {
                    value: *value,
                    name: [0; 32],
                };
                copy_truncated(&mut mi.name, name);
                mi
            })
            .collect();

        let mut raw = uvc_xu_control_mapping {
            id: map.id,
            name: [0; 32],
            entity: map.entity,
            selector: map.selector,
            size: map.size_bits,
            offset: map.offset_bits,
            v4l2_type: map.v4l2_type,
            data_type: map.data_type,
            menu_info: if menu.is_empty() {
                std::ptr::null_mut()
            } else {
                menu.as_mut_ptr()
            },
            menu_count: menu.len() as u32,
            reserved: [0; 4],
        };
        copy_truncated(&mut raw.name, map.name);

        match unsafe { uvcioc_ctrl_map(self.0.as_raw_fd(), &mut raw) } {
            Ok(_) => Ok(true),
            Err(Errno::EEXIST) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Copy a string into a fixed NUL-terminated byte array, truncating on a
/// character boundary.
fn copy_truncated(dst: &mut [u8], src: &str) {
    let max = dst.len() - 1;
    let mut end = src.len().min(max);
    while !src.is_char_boundary(end) {
        end -= 1;
    }
    dst[..end].copy_from_slice(&src.as_bytes()[..end]);
}

fn nul_terminated(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Set on metadata nodes, which cannot carry the UVC control ioctls we need.
const V4L2_CAP_META_CAPTURE: u32 = 0x00800000;

pub(crate) fn open_camera(hint: &str) -> Result<CameraHandle, crate::Error> {
    for path in [hint.to_owned(), "/dev/".to_owned() + hint] {
        if let Ok(file) = File::open(&path) {
            // Only character devices: File::open also succeeds on
            // directories and regular files, which fail confusingly at
            // ioctl time.
            if file
                .metadata()
                .map(|m| m.file_type().is_char_device())
                .unwrap_or(false)
            {
                return Ok(file.into());
            }
        }
    }

    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    for path in glob_with("/dev/video*", options).unwrap().flatten() {
        if let Ok(device) = File::open(&path) {
            if let Ok(caps) = v4l2_capability::new(&device) {
                let card = nul_terminated(&caps.card);
                let bus = nul_terminated(&caps.bus_info);
                if (card.contains(hint) || bus.contains(hint))
                    && (caps.device_caps & V4L2_CAP_META_CAPTURE == 0)
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
    fn new(dev: &std::fs::File) -> Result<Self, Errno> {
        let mut query = [v4l2_capability::default()];
        unsafe { ioctl_videoc_querycap(dev.as_raw_fd(), &mut query) }?;
        Ok(query[0])
    }
}

// VIDIOC_QUERYCAP = _IOR('V', 0, struct v4l2_capability)
ioctl_read_buf!(ioctl_videoc_querycap, b'V', 0x00, v4l2_capability);

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
    value: u32,
    name: [u8; 32],
}

/// Kernel struct from linux/uvcvideo.h; `entity` is the GUID in raw
/// byte order, `size` and `offset` are in bits.
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct uvc_xu_control_mapping {
    id: u32,
    name: [u8; 32],
    entity: [u8; 16],
    selector: u8,
    size: u8,
    offset: u8,
    v4l2_type: u32,
    data_type: u32,
    menu_info: *mut uvc_menu_info,
    menu_count: u32,
    reserved: [u32; 4],
}

// Defined in linux/uvcvideo.h:
// UVCIOC_CTRL_MAP   = _IOWR('u', 0x20, struct uvc_xu_control_mapping)
// UVCIOC_CTRL_QUERY = _IOWR('u', 0x21, struct uvc_xu_control_query)
const UVCIOC_MAGIC: u8 = b'u';
ioctl_readwrite!(uvcioc_ctrl_map, UVCIOC_MAGIC, 0x20, uvc_xu_control_mapping);
ioctl_readwrite_buf!(uvcioc_ctrl_query, UVCIOC_MAGIC, 0x21, uvc_xu_control_query);

/* A.8. Video Class-Specific Request Codes */
pub const UVC_SET_CUR: u8 = 0x01;
pub const UVC_GET_CUR: u8 = 0x81;
pub const UVC_GET_LEN: u8 = 0x85;

// ---- Standard V4L2 control ioctls ----

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

impl v4l2_queryctrl {
    pub fn name_string(&self) -> String {
        nul_terminated(&self.name)
    }
}

/// Kernel struct from linux/uvcvideo.h; packed there because of the
/// union { name[32]; value } member, so 44 bytes. We only use the name.
#[repr(C, packed)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone)]
pub struct v4l2_querymenu {
    pub id: u32,
    pub index: u32,
    pub name: [u8; 32],
    pub reserved: u32,
}

pub const V4L2_CTRL_FLAG_DISABLED: u32 = 0x0001;
pub const V4L2_CTRL_FLAG_NEXT_CTRL: u32 = 0x80000000;

// VIDIOC_G_CTRL    = _IOWR('V', 27, struct v4l2_control)
// VIDIOC_S_CTRL    = _IOWR('V', 28, struct v4l2_control)
// VIDIOC_QUERYCTRL = _IOWR('V', 36, struct v4l2_queryctrl)
// VIDIOC_QUERYMENU = _IOWR('V', 37, struct v4l2_querymenu)
ioctl_readwrite!(vidioc_g_ctrl, b'V', 27, v4l2_control);
ioctl_readwrite!(vidioc_s_ctrl, b'V', 28, v4l2_control);
ioctl_readwrite!(vidioc_queryctrl, b'V', 36, v4l2_queryctrl);
ioctl_readwrite!(vidioc_querymenu, b'V', 37, v4l2_querymenu);

// Standard V4L2 Camera Control IDs
// V4L2_CID_CAMERA_CLASS_BASE = 0x009A0900
pub const V4L2_CID_PAN_ABSOLUTE: u32 = 0x009A0908;
pub const V4L2_CID_TILT_ABSOLUTE: u32 = 0x009A0909;
pub const V4L2_CID_ZOOM_ABSOLUTE: u32 = 0x009A090D;
// Speed controls: uvcvideo maps the UVC PANTILT_RELATIVE control
// (speed-based continuous motion, not a relative step) to these ids.
pub const V4L2_CID_PAN_SPEED: u32 = 0x009A0920;
pub const V4L2_CID_TILT_SPEED: u32 = 0x009A0921;

#[cfg(test)]
mod abi {
    //! Layout and ioctl-number checks against the real kernel headers.
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn struct_layouts() {
        assert_eq!(size_of::<uvc_xu_control_mapping>(), 96);
        assert_eq!(offset_of!(uvc_xu_control_mapping, id), 0);
        assert_eq!(offset_of!(uvc_xu_control_mapping, name), 4);
        assert_eq!(offset_of!(uvc_xu_control_mapping, entity), 36);
        assert_eq!(offset_of!(uvc_xu_control_mapping, selector), 52);
        assert_eq!(offset_of!(uvc_xu_control_mapping, size), 53);
        assert_eq!(offset_of!(uvc_xu_control_mapping, offset), 54);
        assert_eq!(offset_of!(uvc_xu_control_mapping, v4l2_type), 56);
        assert_eq!(offset_of!(uvc_xu_control_mapping, data_type), 60);
        assert_eq!(offset_of!(uvc_xu_control_mapping, menu_info), 64);
        assert_eq!(offset_of!(uvc_xu_control_mapping, menu_count), 72);
        assert_eq!(offset_of!(uvc_xu_control_mapping, reserved), 76);

        assert_eq!(size_of::<uvc_menu_info>(), 36);
        assert_eq!(size_of::<uvc_xu_control_query>(), 16);
        assert_eq!(size_of::<v4l2_control>(), 8);
        assert_eq!(size_of::<v4l2_queryctrl>(), 68);
        assert_eq!(size_of::<v4l2_querymenu>(), 44);
        assert_eq!(size_of::<v4l2_capability>(), 104);
    }

    #[test]
    fn ioctl_numbers() {
        assert_eq!(
            nix::request_code_readwrite!(b'u', 0x20, size_of::<uvc_xu_control_mapping>()),
            0xc060_7520
        );
        assert_eq!(
            nix::request_code_readwrite!(b'u', 0x21, size_of::<uvc_xu_control_query>()),
            0xc010_7521
        );
        assert_eq!(
            nix::request_code_readwrite!(b'V', 36, size_of::<v4l2_queryctrl>()),
            0xc044_5624
        );
        assert_eq!(
            nix::request_code_readwrite!(b'V', 37, size_of::<v4l2_querymenu>()),
            0xc02c_5625
        );
        assert_eq!(
            nix::request_code_readwrite!(b'V', 27, size_of::<v4l2_control>()),
            0xc008_561b
        );
        assert_eq!(
            nix::request_code_readwrite!(b'V', 28, size_of::<v4l2_control>()),
            0xc008_561c
        );
    }

    #[test]
    fn copy_truncated_bounds() {
        let mut buf = [0u8; 8];
        copy_truncated(&mut buf, "abcdefghij");
        assert_eq!(&buf, b"abcdefg\0");
        let mut buf = [0xFFu8; 4];
        copy_truncated(&mut buf, "ab");
        assert_eq!(&buf[..3], b"ab\xff");
        // Multi-byte chars are not split
        let mut buf = [0u8; 4];
        copy_truncated(&mut buf, "aéé");
        assert_eq!(&buf, b"a\xc3\xa9\0");
    }
}
