use iced::widget::{button, column, row, slider, text, text_input, toggler};
use iced::{executor, window, Alignment, Length};
use iced::{Application, Command, Element, Settings, Theme};
use std::time::Duration;

use tiny2::{AIMode, Camera, CtrlRange, ExposureMode, FOVMode, OBSBotWebCam};

#[derive(Debug, Clone, PartialEq)]
enum Message {
    ChangeTracking(AIMode),
    ChangeHDR(bool),
    ChangeExposure(ExposureMode),
    ChangeFOV(FOVMode),
    TextInput(String),
    TextInput02(String),
    SendCommand,
    SendCommand02,
    HexDump,
    HexDump02,
    DismissError,
    // PTZ messages
    SetPan(i32),
    SetTilt(i32),
    SetZoom(i32),
    PanRelative(i32),
    TiltRelative(i32),
    ZoomRelative(i32),
}

/// Cached range + current value for a PTZ axis.
struct PtzAxis {
    value: i32,
    range: Option<CtrlRange>,
}

impl PtzAxis {
    fn unavailable() -> Self {
        PtzAxis {
            value: 0,
            range: None,
        }
    }

    fn new(value: i32, range: CtrlRange) -> Self {
        PtzAxis {
            value,
            range: Some(range),
        }
    }

    /// Suggested step for arrow-button increments (use control step, or 1/20 of range).
    fn step(&self) -> i32 {
        match &self.range {
            Some(r) if r.step > 0 => r.step,
            Some(r) => ((r.maximum - r.minimum) / 20).max(1),
            None => 1,
        }
    }
}

struct MainPanel {
    camera: Camera,
    tracking: AIMode,
    hdr_on: bool,
    text_input: String,
    text_input_02: String,
    error_message: Option<String>,
    pan: PtzAxis,
    tilt: PtzAxis,
    zoom: PtzAxis,
}

impl Application for MainPanel {
    fn view(&self) -> Element<Message> {
        let mut c = column![
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
            row![
                button("FOV 86°")
                    .on_press(Message::ChangeFOV(FOVMode::Wide))
                    .width(Length::Fill),
                button("FOV 78°")
                    .on_press(Message::ChangeFOV(FOVMode::Normal))
                    .width(Length::Fill),
                button("FOV 65°")
                    .on_press(Message::ChangeFOV(FOVMode::Narrow))
                    .width(Length::Fill),
            ]
            .spacing(10),
            toggler(Some("HDR".to_string()), self.hdr_on, Message::ChangeHDR),
        ]
        .width(Length::Fill)
        .align_items(Alignment::Center)
        .spacing(10)
        .padding(10);

        // Pan/Tilt/Zoom controls
        if let Some(ref range) = self.pan.range {
            let step = self.pan.step();
            c = c.push(
                row![
                    text("Pan").width(Length::Fixed(40.0)),
                    button("<").on_press(Message::PanRelative(-step)),
                    slider(
                        range.minimum..=range.maximum,
                        self.pan.value,
                        Message::SetPan
                    )
                    .step(range.step.max(1))
                    .width(Length::Fill),
                    button(">").on_press(Message::PanRelative(step)),
                    text(format!("{}", self.pan.value)).width(Length::Fixed(60.0)),
                ]
                .spacing(5)
                .align_items(Alignment::Center),
            );
        }

        if let Some(ref range) = self.tilt.range {
            let step = self.tilt.step();
            c = c.push(
                row![
                    text("Tilt").width(Length::Fixed(40.0)),
                    button("v").on_press(Message::TiltRelative(-step)),
                    slider(
                        range.minimum..=range.maximum,
                        self.tilt.value,
                        Message::SetTilt
                    )
                    .step(range.step.max(1))
                    .width(Length::Fill),
                    button("^").on_press(Message::TiltRelative(step)),
                    text(format!("{}", self.tilt.value)).width(Length::Fixed(60.0)),
                ]
                .spacing(5)
                .align_items(Alignment::Center),
            );
        }

        if let Some(ref range) = self.zoom.range {
            let step = self.zoom.step();
            c = c.push(
                row![
                    text("Zoom").width(Length::Fixed(40.0)),
                    button("-").on_press(Message::ZoomRelative(-step)),
                    slider(
                        range.minimum..=range.maximum,
                        self.zoom.value,
                        Message::SetZoom
                    )
                    .step(range.step.max(1))
                    .width(Length::Fill),
                    button("+").on_press(Message::ZoomRelative(step)),
                    text(format!("{}", self.zoom.value)).width(Length::Fixed(60.0)),
                ]
                .spacing(5)
                .align_items(Alignment::Center),
            );
        }

        if self.pan.range.is_none() && self.tilt.range.is_none() && self.zoom.range.is_none() {
            c = c.push(text("PTZ controls not available for this device"));
        }

        c = c.push(
            column![
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
                text(self.tracking),
            ]
            .spacing(10),
        );

        c.into()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ChangeTracking(tracking_type) => {
                self.tracking = tracking_type;
                if let Err(e) = self.camera.set_ai_mode(tracking_type) {
                    self.error_message = Some(format!("Failed to change tracking: {}", e));
                }
                Command::none()
            }
            Message::ChangeHDR(new_mode) => {
                self.hdr_on = new_mode;
                if let Err(e) = self.camera.set_hdr_mode(new_mode) {
                    self.error_message = Some(format!("Failed to change HDR mode: {}", e));
                }
                Command::none()
            }
            Message::ChangeExposure(mode) => {
                if let Err(e) = self.camera.set_exposure_mode(mode) {
                    self.error_message = Some(format!("Failed to change exposure: {}", e));
                }
                Command::none()
            }
            Message::ChangeFOV(value) => {
                if let Err(e) = self.camera.set_fov(value) {
                    self.error_message = Some(format!("Failed to change FOV: {}", e));
                }
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
                match hex::decode(&self.text_input) {
                    Ok(c) => {
                        if let Err(e) = self.camera.send_cmd(0x2, 0x6, &c) {
                            self.error_message = Some(format!("Failed to send command: {}", e));
                        }
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Invalid hex string: {}", e));
                    }
                }
                Command::none()
            }
            Message::SendCommand02 => {
                match hex::decode(&self.text_input_02) {
                    Ok(c) => {
                        if let Err(e) = self.camera.send_cmd(0x2, 0x2, &c) {
                            self.error_message = Some(format!("Failed to send command: {}", e));
                        }
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Invalid hex string: {}", e));
                    }
                }
                Command::none()
            }
            Message::HexDump => {
                if let Err(e) = self.camera.dump() {
                    self.error_message = Some(format!("Failed to dump: {}", e));
                }
                Command::none()
            }
            Message::HexDump02 => {
                if let Err(e) = self.camera.dump_02() {
                    self.error_message = Some(format!("Failed to dump: {}", e));
                }
                Command::none()
            }
            Message::DismissError => {
                self.error_message = None;
                Command::none()
            }
            Message::SetPan(value) => {
                self.pan.value = value;
                if let Err(e) = self.camera.set_pan(value) {
                    self.error_message = Some(format!("Failed to set pan: {}", e));
                }
                Command::none()
            }
            Message::SetTilt(value) => {
                self.tilt.value = value;
                if let Err(e) = self.camera.set_tilt(value) {
                    self.error_message = Some(format!("Failed to set tilt: {}", e));
                }
                Command::none()
            }
            Message::SetZoom(value) => {
                self.zoom.value = value;
                if let Err(e) = self.camera.set_zoom(value) {
                    self.error_message = Some(format!("Failed to set zoom: {}", e));
                }
                Command::none()
            }
            Message::PanRelative(delta) => {
                if let Some(ref range) = self.pan.range {
                    let new_val = (self.pan.value + delta).clamp(range.minimum, range.maximum);
                    self.pan.value = new_val;
                    if let Err(e) = self.camera.set_pan(new_val) {
                        self.error_message = Some(format!("Failed to set pan: {}", e));
                    }
                }
                Command::none()
            }
            Message::TiltRelative(delta) => {
                if let Some(ref range) = self.tilt.range {
                    let new_val = (self.tilt.value + delta).clamp(range.minimum, range.maximum);
                    self.tilt.value = new_val;
                    if let Err(e) = self.camera.set_tilt(new_val) {
                        self.error_message = Some(format!("Failed to set tilt: {}", e));
                    }
                }
                Command::none()
            }
            Message::ZoomRelative(delta) => {
                if let Some(ref range) = self.zoom.range {
                    let new_val = (self.zoom.value + delta).clamp(range.minimum, range.maximum);
                    self.zoom.value = new_val;
                    if let Err(e) = self.camera.set_zoom(new_val) {
                        self.error_message = Some(format!("Failed to set zoom: {}", e));
                    }
                }
                Command::none()
            }
        }
    }

    type Executor = executor::Default;

    type Message = Message;

    type Theme = Theme;

    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let camera = Camera::wait_for("OBSBOT Tiny 2", Duration::from_secs(1));

        let status = match camera.get_status() {
            Ok(s) => s,
            Err(e) => {
                return (
                    MainPanel {
                        camera,
                        tracking: AIMode::NoTracking,
                        hdr_on: false,
                        text_input: String::new(),
                        text_input_02: String::new(),
                        error_message: Some(format!("Failed to get camera status: {}", e)),
                        pan: PtzAxis::unavailable(),
                        tilt: PtzAxis::unavailable(),
                        zoom: PtzAxis::unavailable(),
                    },
                    Command::none(),
                )
            }
        };

        // Query PTZ control ranges and current values.
        // These are standard V4L2 controls and may not be supported by all devices.
        let pan = match (camera.query_pan_range(), camera.get_pan()) {
            (Ok(range), Ok(value)) => PtzAxis::new(value, range),
            _ => PtzAxis::unavailable(),
        };
        let tilt = match (camera.query_tilt_range(), camera.get_tilt()) {
            (Ok(range), Ok(value)) => PtzAxis::new(value, range),
            _ => PtzAxis::unavailable(),
        };
        let zoom = match (camera.query_zoom_range(), camera.get_zoom()) {
            (Ok(range), Ok(value)) => PtzAxis::new(value, range),
            _ => PtzAxis::unavailable(),
        };

        (
            MainPanel {
                camera,
                tracking: status.ai_mode,
                hdr_on: status.hdr_on,
                text_input: String::new(),
                text_input_02: String::new(),
                error_message: None,
                pan,
                tilt,
                zoom,
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
            size: (400, 700),
            resizable: false,
            decorations: true,
            ..Default::default()
        },
        ..Default::default()
    })
}
