use iced::widget::{button, column, row, text, text_input, toggler};
use iced::{executor, window, Alignment, Length};
use iced::{Application, Command, Element, Settings, Theme};

use tiny2::{AIMode, Camera, OBSBotWebCam, ExposureMode};

#[derive(Debug, Clone, PartialEq)]
enum Message {
    ChangeTracking(AIMode),
    ChangeHDR(bool),
    ChangeExposure(ExposureMode),
    TextInput(String),
    TextInput02(String),
    SendCommand,
    SendCommand02,
    HexDump,
    HexDump02,
}

struct MainPanel {
    camera: Camera,
    tracking: AIMode,
    hdr_on: bool,
    text_input: String,
    text_input_02: String,
}

impl Application for MainPanel {
    fn view(&self) -> Element<Message> {
        let c = column![
            button("None").on_press(Message::ChangeTracking(AIMode::NoTracking)),
            button("Normal Tracking").on_press(Message::ChangeTracking(AIMode::NormalTracking)),
            row![
                button("Upper Body")
                    .on_press(Message::ChangeTracking(AIMode::UpperBody))
                    .width(Length::Fill),
                button("Close-up")
                    .on_press(Message::ChangeTracking(AIMode::CloseUp))
                    .width(Length::Fill),
            ]
            .spacing(10),
            row![
                button("Headless")
                    .on_press(Message::ChangeTracking(AIMode::Headless))
                    .width(Length::Fill),
                button("Lower Body")
                    .on_press(Message::ChangeTracking(AIMode::LowerBody))
                    .width(Length::Fill),
            ]
            .spacing(10),
            row![
                button("Desk")
                    .on_press(Message::ChangeTracking(AIMode::DeskMode))
                    .width(Length::Fill),
                button("Whiteboard")
                    .on_press(Message::ChangeTracking(AIMode::Whiteboard))
                    .width(Length::Fill),
            ]
            .spacing(10),
            row![
                button("Hand")
                    .on_press(Message::ChangeTracking(AIMode::Hand))
                    .width(Length::Fill),
                button("Group")
                    .on_press(Message::ChangeTracking(AIMode::Group))
                    .width(Length::Fill),
            ]
            .spacing(10),
            row![
                button("Manual")
                    .on_press(Message::ChangeExposure(ExposureMode::Manual))
                    .width(Length::Fill),
                button("Face")
                    .on_press(Message::ChangeExposure(ExposureMode::Face))
                    .width(Length::Fill),
                button("Global")
                    .on_press(Message::ChangeExposure(ExposureMode::Global))
                    .width(Length::Fill),
            ]
            .spacing(10),
            toggler(
                Some("HDR".to_string()),
                self.hdr_on,
                Message::ChangeHDR
            ),
            text_input("0x06 hex string", &self.text_input)
                .on_input(Message::TextInput)
                .on_submit(Message::SendCommand),
            text_input("0x02 hex string", &self.text_input_02)
                .on_input(Message::TextInput02)
                .on_submit(Message::SendCommand02),
            button("Dump 0x06")
                .on_press(Message::HexDump)
                .width(Length::Fill),
            button("Dump 0x02")
                .on_press(Message::HexDump02)
                .width(Length::Fill),
            text(self.tracking)
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .align_items(Alignment::Center)
        .spacing(10)
        .padding(10)
        .into();
        c
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ChangeTracking(tracking_type) => {
                self.tracking = tracking_type;
                self.camera.set_ai_mode(tracking_type).unwrap();
                Command::none()
            }
            Message::ChangeHDR(new_mode) => {
                self.hdr_on = new_mode;
                self.camera.set_hdr_mode(new_mode).unwrap();
                Command::none()
            }
            Message::ChangeExposure(mode) => {
                self.camera.set_exposure_mode(mode).unwrap();
                Command::none()
            }
            Message::TextInput(s) => {
                self.text_input = s;
                Command::none()
            }
            Message::TextInput02(s) => {
                self.text_input_02 = s;
                Command::none()
            }
            Message::SendCommand => {
                let c = hex::decode(&self.text_input).unwrap();
                self.camera.send_cmd(0x2, 0x6, &c).unwrap();
                Command::none()
            }
            Message::SendCommand02 => {
                let c = hex::decode(&self.text_input_02).unwrap();
                self.camera.send_cmd(0x2, 0x2, &c).unwrap();
                Command::none()
            }
            Message::HexDump => {
                self.camera.dump().unwrap();
                Command::none()
            }
            Message::HexDump02 => {
                self.camera.dump_02().unwrap();
                Command::none()
            }
        }
    }

    type Executor = executor::Default;

    type Message = Message;

    type Theme = Theme;

    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let camera = Camera::new("OBSBOT Tiny 2").unwrap();

        let status = camera.get_status().unwrap();

        (
            MainPanel {
                camera,
                tracking: status.ai_mode,
                hdr_on: status.hdr_on,
                text_input: String::new(),
                text_input_02: String::new(),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "ObsBot Tiny 2 Control Panel".to_string()
    }
}

fn main() -> iced::Result {
    MainPanel::run(Settings {
        window: window::Settings {
            size: (300, 540),
            resizable: false,
            decorations: true,
            ..Default::default()
        },
        ..Default::default()
    })
}
