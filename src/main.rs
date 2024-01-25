// SPDX-License-Identifier: EUPL-1.2

mod usbio;

use errno::Errno;
use iced::widget::{button, column, container};
use iced::{executor, window, Alignment, Length};
use iced::{Application, Command, Element, Settings, Theme};

#[derive(Debug)]
pub struct Camera {
    handle: usbio::CameraHandle,
}

impl Camera {
    pub fn new(hint: &str) -> Result<Self, errno::Errno> {
        match usbio::open_camera(hint) {
            Ok(camera) => Ok(Self { handle: camera }),
            Err(err) => Err(err),
        }
    }

    pub fn info(&self) -> Result<(), Errno> {
        self.handle.info()
    }

    pub fn dump(&self) -> Result<(), Errno> {
        let mut data = [0u8; 60];
        self.get_cur(0x2, 0x6, &mut data)?;
        hexdump::hexdump(&data);
        Ok(())
    }

    fn send_cmd(&self, unit: u8, selector: u8, cmd: &[u8]) -> Result<(), errno::Errno> {
        let mut data = [0u8; 60];
        data[..cmd.len()].copy_from_slice(cmd);

        self.set_cur(unit, selector, &mut data)
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

        println!("{:?}", data);

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum TrackingType {
    None,
    Face,
    FaceUpperBody,
    Whiteboard,
    Group,
    Hand,
    Desk,
}

#[derive(Debug, Clone, Copy, PartialEq)]

enum Message {
    ChangeTracking(TrackingType),
    Succeeded,
}

struct MainPanel {
    camera: Camera,
    tracking: TrackingType,
}

impl Application for MainPanel {
    fn view(&self) -> Element<Message> {
        let c = column![
            button("None").on_press(Message::ChangeTracking(TrackingType::None)),
            button("Face / No Zoom").on_press(Message::ChangeTracking(TrackingType::Face)),
            button("Face / Upper Body")
                .on_press(Message::ChangeTracking(TrackingType::FaceUpperBody)),
            button("Whiteboard").on_press(Message::ChangeTracking(TrackingType::Whiteboard)),
            button("Group").on_press(Message::ChangeTracking(TrackingType::Group)),
            button("Hand").on_press(Message::ChangeTracking(TrackingType::Hand)),
            button("Desk").on_press(Message::ChangeTracking(TrackingType::Desk))
        ]
        .width(Length::Fill)
        .align_items(Alignment::Center)
        .spacing(10);

        container(c)
            .center_x()
            .center_y()
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ChangeTracking(tracking_type) => {
                self.tracking = tracking_type;
                set_tracking_mode(&self.camera, tracking_type);
                Command::none()
            }
            Message::Succeeded => Command::none(),
        }
    }

    type Executor = executor::Default;

    type Message = Message;

    type Theme = Theme;

    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let camera = Camera::new("OBSBOT Tiny 2").unwrap();

        (
            MainPanel {
                camera,
                tracking: TrackingType::None,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "ObsBot Tiny 2 Control Panel".to_string()
    }
}

fn set_tracking_mode(camera: &Camera, mode: TrackingType) {
    println!("{:?}", mode);
    let cmd = match mode {
        TrackingType::None => [0x16, 0x02, 0x00, 0x00],
        TrackingType::Face => [0x16, 0x02, 0x02, 0x00],
        TrackingType::FaceUpperBody => [0x16, 0x02, 0x02, 0x01],
        TrackingType::Desk => [0x16, 0x02, 0x05, 0x00],
        TrackingType::Whiteboard => [0x16, 0x02, 0x04, 0x00],
        TrackingType::Group => [0x16, 0x02, 0x01, 0x00],
        TrackingType::Hand => [0x16, 0x02, 0x03, 0x00],
    };
    camera.send_cmd(0x2, 0x6, &cmd).unwrap();
}

fn main() -> iced::Result {
    MainPanel::run(Settings {
        window: window::Settings {
            size: (300, 500),
            resizable: true,
            decorations: true,
            ..Default::default()
        },
        ..Default::default()
    })
}
