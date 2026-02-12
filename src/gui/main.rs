use iced::widget::{button, column, container, mouse_area, row, text, text_input, toggler};
use iced::{event, time, window, Alignment, Border, Element, Length, Subscription, Task, Theme};
use std::time::Duration;

use tiny2::{AIMode, Camera, ExposureMode, FOVMode, OBSBotWebCam};

#[derive(Debug, Clone, Copy, PartialEq)]
enum PtzAction {
    Pan(i32),
    Tilt(i32),
    Zoom(i32),
}

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
    // PTZ press-and-hold
    StartMove(PtzAction),
    StopMove,
    Tick,
}

/// Step size for a PTZ axis, or None if the control is unavailable.
struct PtzAxis {
    step: Option<i32>,
}

impl PtzAxis {
    fn unavailable() -> Self {
        PtzAxis { step: None }
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
    held_action: Option<PtzAction>,
}

impl MainPanel {
    fn execute_ptz(&mut self, action: PtzAction) {
        let result = match action {
            PtzAction::Pan(d) => self.camera.get_pan().and_then(|v| self.camera.set_pan(v + d)),
            PtzAction::Tilt(d) => self.camera.get_tilt().and_then(|v| self.camera.set_tilt(v + d)),
            PtzAction::Zoom(d) => self.camera.get_zoom().and_then(|v| self.camera.set_zoom(v + d)),
        };
        if let Err(e) = result {
            self.error_message = Some(format!("PTZ error: {}", e));
        }
    }
}

fn boot() -> (MainPanel, Task<Message>) {
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
                    held_action: None,
                },
                Task::none(),
            )
        }
    };

    let step_from_range = |r: tiny2::CtrlRange| -> i32 {
        if r.step > 0 {
            r.step
        } else {
            ((r.maximum - r.minimum) / 20).max(1)
        }
    };
    let pan = PtzAxis {
        step: camera.query_pan_range().ok().map(&step_from_range),
    };
    let tilt = PtzAxis {
        step: camera.query_tilt_range().ok().map(&step_from_range),
    };
    let zoom = PtzAxis {
        step: camera.query_zoom_range().ok().map(&step_from_range),
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
            held_action: None,
        },
        Task::none(),
    )
}

fn update(state: &mut MainPanel, message: Message) -> Task<Message> {
    match message {
        Message::ChangeTracking(tracking_type) => {
            state.tracking = tracking_type;
            if let Err(e) = state.camera.set_ai_mode(tracking_type) {
                state.error_message = Some(format!("Failed to change tracking: {}", e));
            }
        }
        Message::ChangeHDR(new_mode) => {
            state.hdr_on = new_mode;
            if let Err(e) = state.camera.set_hdr_mode(new_mode) {
                state.error_message = Some(format!("Failed to change HDR mode: {}", e));
            }
        }
        Message::ChangeExposure(mode) => {
            if let Err(e) = state.camera.set_exposure_mode(mode) {
                state.error_message = Some(format!("Failed to change exposure: {}", e));
            }
        }
        Message::ChangeFOV(value) => {
            if let Err(e) = state.camera.set_fov(value) {
                state.error_message = Some(format!("Failed to change FOV: {}", e));
            }
        }
        Message::TextInput(s) => {
            state.text_input = s;
        }
        Message::TextInput02(s) => {
            state.text_input_02 = s;
        }
        Message::SendCommand => match hex::decode(&state.text_input) {
            Ok(c) => {
                if let Err(e) = state.camera.send_cmd(0x2, 0x6, &c) {
                    state.error_message = Some(format!("Failed to send command: {}", e));
                }
            }
            Err(e) => {
                state.error_message = Some(format!("Invalid hex string: {}", e));
            }
        },
        Message::SendCommand02 => match hex::decode(&state.text_input_02) {
            Ok(c) => {
                if let Err(e) = state.camera.send_cmd(0x2, 0x2, &c) {
                    state.error_message = Some(format!("Failed to send command: {}", e));
                }
            }
            Err(e) => {
                state.error_message = Some(format!("Invalid hex string: {}", e));
            }
        },
        Message::HexDump => {
            if let Err(e) = state.camera.dump() {
                state.error_message = Some(format!("Failed to dump: {}", e));
            }
        }
        Message::HexDump02 => {
            if let Err(e) = state.camera.dump_02() {
                state.error_message = Some(format!("Failed to dump: {}", e));
            }
        }
        Message::DismissError => {
            state.error_message = None;
        }
        Message::StartMove(action) => {
            state.held_action = Some(action);
            state.execute_ptz(action);
        }
        Message::StopMove => {
            state.held_action = None;
        }
        Message::Tick => {
            if let Some(action) = state.held_action {
                state.execute_ptz(action);
            }
        }
    }
    Task::none()
}

fn view(state: &MainPanel) -> Element<Message> {
    let track_btn = |label: &'static str, mode: AIMode| {
        let style = if state.tracking == mode {
            button::primary
        } else {
            button::secondary
        };
        button(text(label).align_x(Alignment::Center))
            .on_press(Message::ChangeTracking(mode))
            .style(style)
            .width(Length::Fill)
    };

    let mut c = column![
        track_btn("None", AIMode::NoTracking),
        track_btn("Normal Tracking", AIMode::NormalTracking),
        row![
            track_btn("Upper Body", AIMode::UpperBody),
            track_btn("Close-up", AIMode::CloseUp),
        ]
        .spacing(10),
        row![
            track_btn("Headless", AIMode::Headless),
            track_btn("Lower Body", AIMode::LowerBody),
        ]
        .spacing(10),
        row![
            track_btn("Desk", AIMode::DeskMode),
            track_btn("Whiteboard", AIMode::Whiteboard),
        ]
        .spacing(10),
        row![
            track_btn("Hand", AIMode::Hand),
            track_btn("Group", AIMode::Group),
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
        toggler(state.hdr_on)
            .label("HDR")
            .on_toggle(Message::ChangeHDR),
    ]
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .spacing(10)
    .padding(10);

    // Pan/Tilt/Zoom press-and-hold controls
    if state.pan.step.is_some() || state.tilt.step.is_some() || state.zoom.step.is_some() {
        let ptz_btn = |label: &'static str| {
            container(text(label).align_x(Alignment::Center))
                .padding([4, 12])
                .style(|theme: &Theme| {
                    let palette = theme.extended_palette();
                    container::Style {
                        background: Some(palette.primary.weak.color.into()),
                        text_color: Some(palette.primary.weak.text),
                        border: Border {
                            radius: 4.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })
        };

        let mut ptz_row = row![].spacing(10).align_y(Alignment::Center);

        if let Some(step) = state.pan.step {
            ptz_row = ptz_row.push(
                row![
                    mouse_area(ptz_btn("<"))
                        .on_press(Message::StartMove(PtzAction::Pan(-step)))
                        .on_release(Message::StopMove),
                    mouse_area(ptz_btn(">"))
                        .on_press(Message::StartMove(PtzAction::Pan(step)))
                        .on_release(Message::StopMove),
                ]
                .spacing(5),
            );
        }

        if let Some(step) = state.tilt.step {
            ptz_row = ptz_row.push(
                row![
                    mouse_area(ptz_btn("v"))
                        .on_press(Message::StartMove(PtzAction::Tilt(-step)))
                        .on_release(Message::StopMove),
                    mouse_area(ptz_btn("^"))
                        .on_press(Message::StartMove(PtzAction::Tilt(step)))
                        .on_release(Message::StopMove),
                ]
                .spacing(5),
            );
        }

        if let Some(step) = state.zoom.step {
            ptz_row = ptz_row.push(
                row![
                    mouse_area(ptz_btn("-"))
                        .on_press(Message::StartMove(PtzAction::Zoom(-step)))
                        .on_release(Message::StopMove),
                    mouse_area(ptz_btn("+"))
                        .on_press(Message::StartMove(PtzAction::Zoom(step)))
                        .on_release(Message::StopMove),
                ]
                .spacing(5),
            );
        }

        c = c.push(ptz_row);
    } else {
        c = c.push(text("PTZ controls not available for this device"));
    }

    c = c.push(
        column![
            text_input("0x06 hex string", &state.text_input)
                .on_input(Message::TextInput)
                .on_submit(Message::SendCommand),
            text_input("0x02 hex string", &state.text_input_02)
                .on_input(Message::TextInput02)
                .on_submit(Message::SendCommand02),
            button("Dump 0x06")
                .on_press(Message::HexDump)
                .width(Length::Fill),
            button("Dump 0x02")
                .on_press(Message::HexDump02)
                .width(Length::Fill),
        ]
        .spacing(10),
    );

    c.into()
}

fn stop_on_mouse_release(
    event: iced::Event,
    _status: event::Status,
    _id: window::Id,
) -> Option<Message> {
    if let iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) =
        event
    {
        Some(Message::StopMove)
    } else {
        None
    }
}

fn subscription(state: &MainPanel) -> Subscription<Message> {
    let tick = if state.held_action.is_some() {
        time::every(Duration::from_millis(150)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    };
    Subscription::batch(vec![tick, event::listen_with(stop_on_mouse_release)])
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        .subscription(subscription)
        .window_size((400.0, 700.0))
        .resizable(false)
        .run()
}
