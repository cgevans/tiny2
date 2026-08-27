// SPDX-License-Identifier: EUPL-1.2

//! obsbot-gui: a small iced control panel for the OBSBot Tiny 2.
//! The behaviour notes in this file are LLM-drafted (Claude).

use iced::widget::{button, column, container, mouse_area, row, text, text_input, toggler};
use iced::{
    event, time, window, Alignment, Border, Element, Length, Subscription, Task, Theme,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tiny2::{AIMode, Camera, CameraStatus, ExposureMode, FOVMode, OBSBotWebCam};

/// Set from --debug flag at startup; controls debug UI and verbose logging.
static DEBUG: AtomicBool = AtomicBool::new(false);

fn debug_mode() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

const CAMERA_HINT: &str = "OBSBOT Tiny 2";

/// Speeds for the press-and-hold pan/tilt buttons, in degrees/second.
const PAN_SPEED_DPS: f32 = 30.0;
const TILT_SPEED_DPS: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum PtzAction {
    Pan(i32),
    Tilt(i32),
    Zoom(i32),
}

#[derive(Debug, Clone)]
enum Message {
    CameraReady(Arc<Camera>),
    RecallPreset(u8),
    Recenter,
    ChangeTracking(AIMode),
    ChangeHDR(bool),
    ChangeMirror(bool),
    ChangeExposure(ExposureMode),
    ChangeFOV(FOVMode),
    Wake,
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
    RefreshStatus,
}

/// Range and step for a PTZ axis, or None if the control is unavailable.
struct PtzAxis {
    range: Option<(i32, i32, i32)>, // (min, max, step)
}

impl PtzAxis {
    fn from_range(r: Result<tiny2::CtrlRange, tiny2::Error>) -> Self {
        PtzAxis {
            range: r.ok().map(|r| {
                let step = if r.step > 0 {
                    r.step
                } else {
                    ((r.maximum - r.minimum) / 20).max(1)
                };
                (r.minimum, r.maximum, step)
            }),
        }
    }

    fn step(&self) -> Option<i32> {
        self.range.map(|(_, _, s)| s)
    }
}

struct MainPanel {
    camera: Option<Arc<Camera>>,
    /// (slot, name) of stored gimbal presets, loaded at connect.
    presets: Vec<(u8, String)>,
    tracking: AIMode,
    hdr_on: bool,
    mirror_on: bool,
    fov: Option<FOVMode>,
    sleeping: bool,
    text_input: String,
    text_input_02: String,
    error_message: Option<String>,
    pan: PtzAxis,
    tilt: PtzAxis,
    zoom: PtzAxis,
    held_action: Option<PtzAction>,
    /// Set after a successful control write; status polling is paused
    /// until then, because the lagging status block would briefly revert
    /// the UI to the pre-command state.
    hold_refresh_until: Option<Instant>,
}

impl MainPanel {
    fn disconnected() -> Self {
        MainPanel {
            camera: None,
            presets: Vec::new(),
            tracking: AIMode::NoTracking,
            hdr_on: false,
            mirror_on: false,
            fov: None,
            sleeping: false,
            text_input: String::new(),
            text_input_02: String::new(),
            error_message: None,
            pan: PtzAxis { range: None },
            tilt: PtzAxis { range: None },
            zoom: PtzAxis { range: None },
            held_action: None,
            hold_refresh_until: None,
        }
    }

    fn apply_status(&mut self, status: &CameraStatus) {
        // ai_mode is None while the firmware reports a transient code
        // (it briefly does while switching modes); keep the last known.
        if let Some(mode) = status.ai_mode {
            self.tracking = mode;
        }
        self.hdr_on = status.hdr_on;
        self.mirror_on = status.mirrored;
        self.fov = status.fov;
        self.sleeping = status.sleeping;
    }

    fn report(&mut self, what: &str, result: Result<(), tiny2::Error>) {
        match result {
            Ok(()) => {
                self.hold_refresh_until = Some(Instant::now() + Duration::from_secs(4));
            }
            Err(e) => {
                self.error_message = Some(format!("{}: {}", what, e));
            }
        }
    }

    fn execute_ptz(&mut self, action: PtzAction) {
        let Some(camera) = &self.camera else { return };
        let clamp = |axis: &PtzAxis, v: i32| match axis.range {
            Some((min, max, _)) => v.clamp(min, max),
            None => v,
        };
        // Only zoom still steps the absolute control: its value moves
        // only through V4L2, so the driver's cache stays correct for it.
        let result = match action {
            PtzAction::Zoom(d) => camera
                .get_zoom()
                .and_then(|v| camera.set_zoom(clamp(&self.zoom, v + d))),
            PtzAction::Pan(_) | PtzAction::Tilt(_) => return,
        };
        if let Err(e) = result {
            self.error_message = Some(format!("PTZ error: {}", e));
        }
    }
}

fn wait_for_camera() -> Task<Message> {
    Task::perform(
        async {
            loop {
                // One short open attempt per iteration, with the waiting in
                // an async sleep: closing the window mid-wait just drops
                // the task instead of hanging shutdown.
                match tokio::task::spawn_blocking(|| Camera::new(CAMERA_HINT)).await {
                    Ok(Ok(mut camera)) => {
                        camera.set_verbose(debug_mode());
                        break camera;
                    }
                    _ => tokio::time::sleep(Duration::from_secs(1)).await,
                }
            }
        },
        |camera| Message::CameraReady(Arc::new(camera)),
    )
}

fn boot() -> (MainPanel, Task<Message>) {
    (MainPanel::disconnected(), wait_for_camera())
}

fn update(state: &mut MainPanel, message: Message) -> Task<Message> {
    match message {
        Message::CameraReady(camera) => {
            state.pan = PtzAxis::from_range(camera.query_pan_range());
            state.tilt = PtzAxis::from_range(camera.query_tilt_range());
            state.zoom = PtzAxis::from_range(camera.query_zoom_range());
            match camera.get_status() {
                Ok(status) => state.apply_status(&status),
                Err(e) => {
                    state.error_message = Some(format!("Failed to get camera status: {}", e))
                }
            }
            state.presets = camera
                .preset_list()
                .map(|l| l.into_iter().map(|p| (p.slot, p.name)).collect())
                .unwrap_or_default();
            state.camera = Some(camera);
        }
        Message::ChangeTracking(tracking_type) => {
            if let Some(camera) = &state.camera {
                let result = camera.set_ai_mode(tracking_type);
                if result.is_ok() {
                    state.tracking = tracking_type;
                }
                state.report("Failed to change tracking", result);
            }
        }
        Message::ChangeHDR(new_mode) => {
            if let Some(camera) = &state.camera {
                let result = camera.set_hdr_mode(new_mode);
                if result.is_ok() {
                    state.hdr_on = new_mode;
                }
                state.report("Failed to change HDR mode", result);
            }
        }
        Message::ChangeMirror(new_mode) => {
            if let Some(camera) = &state.camera {
                let result = camera.set_mirror(new_mode);
                if result.is_ok() {
                    state.mirror_on = new_mode;
                }
                state.report("Failed to change mirror", result);
            }
        }
        Message::ChangeExposure(mode) => {
            if let Some(camera) = &state.camera {
                let result = camera.set_exposure_mode(mode);
                state.report("Failed to change exposure", result);
            }
        }
        Message::ChangeFOV(value) => {
            if let Some(camera) = &state.camera {
                let result = camera.set_fov(value);
                if result.is_ok() {
                    state.fov = Some(value);
                }
                state.report("Failed to change FOV", result);
            }
        }
        Message::RecallPreset(slot) => {
            if let Some(camera) = &state.camera {
                let result = camera.preset_recall(slot);
                state.report("Failed to recall preset", result);
            }
        }
        Message::Recenter => {
            if let Some(camera) = &state.camera {
                let result = camera.gimbal_recenter();
                state.report("Failed to recenter", result);
            }
        }
        Message::Wake => {
            if let Some(camera) = &state.camera {
                let result = camera.wake();
                state.report("Failed to wake camera", result);
            }
        }
        Message::TextInput(s) => {
            state.text_input = s;
        }
        Message::TextInput02(s) => {
            state.text_input_02 = s;
        }
        Message::SendCommand => {
            if let Some(camera) = &state.camera {
                match hex::decode(&state.text_input) {
                    Ok(c) => {
                        let result = camera.send_cmd(0x2, 0x6, &c);
                        state.report("Failed to send command", result);
                    }
                    Err(e) => {
                        state.error_message = Some(format!("Invalid hex string: {}", e));
                    }
                }
            }
        }
        Message::SendCommand02 => {
            if let Some(camera) = &state.camera {
                match hex::decode(&state.text_input_02) {
                    Ok(c) => {
                        let result = camera.send_cmd(0x2, 0x2, &c);
                        state.report("Failed to send command", result);
                    }
                    Err(e) => {
                        state.error_message = Some(format!("Invalid hex string: {}", e));
                    }
                }
            }
        }
        Message::HexDump => {
            if let Some(camera) = &state.camera {
                let result = camera.dump();
                state.report("Failed to dump", result);
            }
        }
        Message::HexDump02 => {
            if let Some(camera) = &state.camera {
                let result = camera.dump_02();
                state.report("Failed to dump", result);
            }
        }
        Message::DismissError => {
            state.error_message = None;
        }
        Message::StartMove(action) => {
            state.held_action = Some(action);
            match action {
                // Pan/tilt hold-buttons drive the gimbal at a speed and
                // stop on release; stepping the position control instead
                // would jump, since the driver's position cache goes stale
                // after tracking or manual moves.
                PtzAction::Pan(d) => {
                    if let Some(camera) = &state.camera {
                        let result =
                            camera.gimbal_speed(d.signum() as f32 * PAN_SPEED_DPS, 0.0);
                        state.report("PTZ error", result);
                    }
                }
                // "^" sends a positive delta (V4L2 tilt-up); view-up is
                // negative pitch.
                PtzAction::Tilt(d) => {
                    if let Some(camera) = &state.camera {
                        let result =
                            camera.gimbal_speed(0.0, -d.signum() as f32 * TILT_SPEED_DPS);
                        state.report("PTZ error", result);
                    }
                }
                PtzAction::Zoom(_) => state.execute_ptz(action),
            }
        }
        Message::StopMove => {
            if matches!(
                state.held_action,
                Some(PtzAction::Pan(_)) | Some(PtzAction::Tilt(_))
            ) {
                if let Some(camera) = &state.camera {
                    let result = camera.gimbal_stop();
                    state.report("PTZ error", result);
                }
            }
            state.held_action = None;
        }
        Message::Tick => {
            // Only zoom needs stepping; pan/tilt motion continues on its
            // own until StopMove.
            if let Some(action @ PtzAction::Zoom(_)) = state.held_action {
                state.execute_ptz(action);
            }
        }
        Message::RefreshStatus => {
            // Sync with changes made elsewhere, but not right after our
            // own command, whose effect can take a couple of seconds to
            // appear in the status block.
            if let Some(until) = state.hold_refresh_until {
                if Instant::now() < until {
                    return Task::none();
                }
                state.hold_refresh_until = None;
            }
            if let Some(camera) = &state.camera {
                match camera.get_status() {
                    Ok(status) => state.apply_status(&status),
                    // Unplugged: drop the camera and wait for a replug.
                    Err(tiny2::Error::Usb(nix::errno::Errno::ENODEV)) => {
                        *state = MainPanel::disconnected();
                        state.error_message = Some("Camera disconnected".to_string());
                        return wait_for_camera();
                    }
                    // Other poll errors are ignored: transient failures
                    // would only spam the error banner.
                    Err(_) => {}
                }
            }
        }
    }
    Task::none()
}

fn view(state: &MainPanel) -> Element<'_, Message> {
    if state.camera.is_none() {
        return container(
            column![
                text("Waiting for camera...").size(20),
                text(format!("Looking for \"{}\"", CAMERA_HINT)),
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into();
    }

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

    let fov_btn = |label: &'static str, mode: FOVMode| {
        let style = if state.fov == Some(mode) {
            button::primary
        } else {
            button::secondary
        };
        button(text(label).align_x(Alignment::Center))
            .on_press(Message::ChangeFOV(mode))
            .style(style)
            .width(Length::Fill)
    };

    let mut c = column![]
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .spacing(10)
        .padding(10);

    if let Some(error) = &state.error_message {
        c = c.push(
            container(
                row![
                    text(error.clone()).width(Length::Fill),
                    button(text("x")).on_press(Message::DismissError),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .padding(8)
            .width(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.danger.weak.color.into()),
                    text_color: Some(palette.danger.weak.text),
                    border: Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
        );
    }

    if state.sleeping {
        c = c.push(
            container(
                row![
                    text("Camera is asleep; most controls are ignored.").width(Length::Fill),
                    button(text("Wake")).on_press(Message::Wake),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .padding(8)
            .width(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.background.weak.color.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
        );
    }

    c = c
        .push(track_btn("None", AIMode::NoTracking))
        .push(track_btn("Normal Tracking", AIMode::NormalTracking))
        .push(
            row![
                track_btn("Upper Body", AIMode::UpperBody),
                track_btn("Close-up", AIMode::CloseUp),
            ]
            .spacing(10),
        )
        .push(
            row![
                track_btn("Headless", AIMode::Headless),
                track_btn("Lower Body", AIMode::LowerBody),
            ]
            .spacing(10),
        )
        .push(
            row![
                track_btn("Desk", AIMode::DeskMode),
                track_btn("Whiteboard", AIMode::Whiteboard),
            ]
            .spacing(10),
        )
        .push(
            row![
                track_btn("Hand", AIMode::Hand),
                track_btn("Group", AIMode::Group),
            ]
            .spacing(10),
        )
        .push(
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
        )
        .push(
            row![
                fov_btn("FOV 86°", FOVMode::Wide),
                fov_btn("FOV 78°", FOVMode::Normal),
                fov_btn("FOV 65°", FOVMode::Narrow),
            ]
            .spacing(10),
        )
        .push(
            row![
                toggler(state.hdr_on)
                    .label("HDR")
                    .on_toggle(Message::ChangeHDR),
                toggler(state.mirror_on)
                    .label("Mirror")
                    .on_toggle(Message::ChangeMirror),
            ]
            .spacing(20),
        );

    // Pan/Tilt/Zoom press-and-hold controls
    if state.pan.step().is_some() || state.tilt.step().is_some() || state.zoom.step().is_some() {
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

        if let Some(step) = state.pan.step() {
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

        if let Some(step) = state.tilt.step() {
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

        if let Some(step) = state.zoom.step() {
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

    {
        let mut gimbal_row = row![].spacing(10);
        for (slot, name) in &state.presets {
            let label = if name.is_empty() {
                format!("Preset {}", slot)
            } else {
                name.clone()
            };
            gimbal_row = gimbal_row.push(
                button(text(label).align_x(Alignment::Center))
                    .on_press(Message::RecallPreset(*slot))
                    .style(button::secondary)
                    .width(Length::Fill),
            );
        }
        gimbal_row = gimbal_row.push(
            button(text("Center").align_x(Alignment::Center))
                .on_press(Message::Recenter)
                .style(button::secondary)
                .width(Length::Fill),
        );
        c = c.push(gimbal_row);
    }

    if debug_mode() {
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
    }

    c.into()
}

fn stop_on_mouse_release(
    event: iced::Event,
    _status: event::Status,
    _id: window::Id,
) -> Option<Message> {
    if let iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) = event
    {
        Some(Message::StopMove)
    } else {
        None
    }
}

fn subscription(state: &MainPanel) -> Subscription<Message> {
    let tick = if matches!(state.held_action, Some(PtzAction::Zoom(_))) {
        time::every(Duration::from_millis(150)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    };
    let refresh = if state.camera.is_some() {
        time::every(Duration::from_secs(2)).map(|_| Message::RefreshStatus)
    } else {
        Subscription::none()
    };
    Subscription::batch(vec![tick, refresh, event::listen_with(stop_on_mouse_release)])
}

fn main() -> iced::Result {
    DEBUG.store(
        std::env::args().any(|a| a == "--debug"),
        Ordering::Relaxed,
    );

    let window_height = if debug_mode() { 680.0 } else { 530.0 };

    iced::application(boot, update, view)
        .title("OBSBot Tiny 2")
        .subscription(subscription)
        .window_size((400.0, window_height))
        .run()
}
