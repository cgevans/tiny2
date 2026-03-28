use iced::widget::{
    button, canvas, column, container, mouse_area, row, scrollable, text, text_input, toggler,
    tooltip,
};
use iced::{
    event, keyboard, mouse, time, window, Alignment, Border, Color, Element, Length, Point,
    Rectangle, Subscription, Task, Theme,
};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tiny2::{AIMode, Camera, ExposureMode, FOVMode, OBSBotWebCam};

/// Set from --debug flag at startup; controls debug UI and verbose logging.
static mut DEBUG: bool = false;

fn debug_mode() -> bool {
    // SAFETY: only written once in main() before iced starts.
    unsafe { DEBUG }
}

// ==================== Camera device discovery ====================

fn find_obsbot_capture_device() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/video4linux/").ok()?;
    let mut candidates: Vec<(String, String)> = Vec::new();

    for entry in entries.flatten() {
        let name_path = entry.path().join("name");
        if let Ok(name) = std::fs::read_to_string(&name_path) {
            if name.trim().contains("OBSBOT Tiny 2") {
                let dev_name = entry.file_name().to_str()?.to_string();
                let index_path = entry.path().join("index");
                let index = std::fs::read_to_string(index_path)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                candidates.push((dev_name, index));
            }
        }
    }

    // Prefer index 0 (capture device) over metadata devices
    candidates.sort_by(|a, b| a.1.cmp(&b.1));
    candidates
        .first()
        .map(|(name, _)| format!("/dev/{}", name))
}

fn spawn_preview(device: &str) -> Result<Child, String> {
    // Try mpv (best low-latency)
    if let Ok(child) = Command::new("mpv")
        .args([
            "--no-osc",
            "--profile=low-latency",
            "--title=OBSBOT Tiny 2 Preview",
            "--geometry=640x480",
            device,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        return Ok(child);
    }

    // Try ffplay
    if let Ok(child) = Command::new("ffplay")
        .args([
            "-window_title",
            "OBSBOT Tiny 2 Preview",
            "-x",
            "640",
            "-y",
            "480",
            "-i",
            device,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        return Ok(child);
    }

    // Try vlc
    if let Ok(child) = Command::new("vlc")
        .args(["--title", "OBSBOT Tiny 2 Preview", device])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        return Ok(child);
    }

    Err("No video player found. Install mpv, ffplay, or vlc.".to_string())
}

// ==================== Messages & State ====================

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
    // PTZ press-and-hold (zoom buttons)
    StartMove(PtzAction),
    StopMove,
    Tick,
    // Joystick
    JoystickDrag(f32, f32),
    JoystickRelease,
    JoystickDoubleClick,
    // Scroll zoom
    ScrollZoom(f32),
    // Debug values panel
    ToggleValuesPanel,
    // Keyboard PTZ
    KeyboardMove(i32, i32),
    KeyboardZoom(i32),
    KeyboardDeactivate,
    // Camera preview
    TogglePreview,
}

struct PtzAxis {
    step: Option<i32>,
    min: i32,
    max: i32,
}

impl PtzAxis {
    fn unavailable() -> Self {
        PtzAxis { step: None, min: 0, max: 0 }
    }

    fn clamp(&self, value: i32) -> i32 {
        value.clamp(self.min, self.max)
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
    // Joystick
    joystick_pos: (f32, f32),
    joystick_active: bool,
    // Keyboard control
    keyboard_focused: bool,
    // Live values
    show_values: bool,
    current_pan: Option<i32>,
    current_tilt: Option<i32>,
    current_zoom: Option<i32>,
    // Camera preview
    preview_process: Option<Child>,
}

impl Drop for MainPanel {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.preview_process {
            let _ = child.kill();
        }
    }
}

impl MainPanel {
    fn execute_ptz(&mut self, action: PtzAction) {
        let result = match action {
            PtzAction::Pan(d) => self
                .camera
                .get_pan()
                .and_then(|v| self.camera.set_pan(self.pan.clamp(v + d))),
            PtzAction::Tilt(d) => self
                .camera
                .get_tilt()
                .and_then(|v| self.camera.set_tilt(self.tilt.clamp(v + d))),
            PtzAction::Zoom(d) => self
                .camera
                .get_zoom()
                .and_then(|v| self.camera.set_zoom(self.zoom.clamp(v + d))),
        };
        if let Err(e) = result {
            self.error_message = Some(format!("PTZ error: {}", e));
        }
    }

    fn refresh_values(&mut self) {
        self.current_pan = self.camera.get_pan().ok();
        self.current_tilt = self.camera.get_tilt().ok();
        self.current_zoom = self.camera.get_zoom().ok();
    }

    fn is_preview_running(&mut self) -> bool {
        if let Some(ref mut child) = self.preview_process {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process exited
                    self.preview_process = None;
                    false
                }
                Ok(None) => true, // Still running
                Err(_) => {
                    self.preview_process = None;
                    false
                }
            }
        } else {
            false
        }
    }
}

// ==================== Joystick Canvas ====================

struct JoystickState {
    dragging: bool,
    last_click: Option<Instant>,
}

impl Default for JoystickState {
    fn default() -> Self {
        JoystickState {
            dragging: false,
            last_click: None,
        }
    }
}

struct JoystickProgram {
    knob_x: f32,
    knob_y: f32,
    keyboard_focused: bool,
}

impl canvas::Program<Message> for JoystickProgram {
    type State = JoystickState;

    fn update(
        &self,
        state: &mut JoystickState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - 4.0;

        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let dx = pos.x - center.x;
                    let dy = pos.y - center.y;
                    if (dx * dx + dy * dy).sqrt() <= radius {
                        // Double-click detection
                        let now = Instant::now();
                        if let Some(last) = state.last_click {
                            if now.duration_since(last) < Duration::from_millis(350) {
                                state.last_click = None;
                                return Some(canvas::Action::publish(
                                    Message::JoystickDoubleClick,
                                ));
                            }
                        }
                        state.last_click = Some(now);

                        state.dragging = true;
                        let nx = (dx / radius).clamp(-1.0, 1.0);
                        let ny = (-dy / radius).clamp(-1.0, 1.0);
                        return Some(canvas::Action::publish(Message::JoystickDrag(nx, ny)));
                    }
                }
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.dragging {
                    if let Some(pos) = cursor.position_in(bounds) {
                        let dx = pos.x - center.x;
                        let dy = pos.y - center.y;
                        let nx = (dx / radius).clamp(-1.0, 1.0);
                        let ny = (-dy / radius).clamp(-1.0, 1.0);
                        return Some(canvas::Action::publish(Message::JoystickDrag(nx, ny)));
                    } else {
                        state.dragging = false;
                        return Some(canvas::Action::publish(Message::JoystickRelease));
                    }
                }
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.dragging {
                    state.dragging = false;
                    return Some(canvas::Action::publish(Message::JoystickRelease));
                }
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &JoystickState,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - 4.0;

        // Background circle
        frame.fill(
            &canvas::Path::circle(center, radius),
            Color::from_rgb(0.15, 0.15, 0.22),
        );

        // Outer ring - green when keyboard focused
        let ring_color = if self.keyboard_focused {
            Color::from_rgba(0.3, 1.0, 0.4, 0.7)
        } else {
            Color::from_rgba(0.4, 0.65, 1.0, 0.4)
        };
        let ring_width = if self.keyboard_focused { 3.0 } else { 2.0 };
        frame.stroke(
            &canvas::Path::circle(center, radius),
            canvas::Stroke::default()
                .with_color(ring_color)
                .with_width(ring_width),
        );

        // Crosshair lines
        let cross_color = Color::from_rgba(1.0, 1.0, 1.0, 0.12);
        let cross_stroke = canvas::Stroke::default()
            .with_color(cross_color)
            .with_width(1.0);
        frame.stroke(
            &canvas::Path::new(|b| {
                b.move_to(Point::new(center.x - radius, center.y));
                b.line_to(Point::new(center.x + radius, center.y));
            }),
            cross_stroke,
        );
        frame.stroke(
            &canvas::Path::new(|b| {
                b.move_to(Point::new(center.x, center.y - radius));
                b.line_to(Point::new(center.x, center.y + radius));
            }),
            cross_stroke,
        );

        // Arrow indicators
        let arrow_color = if self.keyboard_focused {
            Color::from_rgba(0.3, 1.0, 0.4, 0.5)
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, 0.25)
        };
        let arrow_stroke = canvas::Stroke::default()
            .with_color(arrow_color)
            .with_width(2.0);
        let arrow_len = 10.0;
        let arrow_dist = radius - 18.0;

        // Left arrow
        frame.stroke(
            &canvas::Path::new(|b| {
                b.move_to(Point::new(center.x - arrow_dist, center.y));
                b.line_to(Point::new(
                    center.x - arrow_dist + arrow_len,
                    center.y - 5.0,
                ));
                b.move_to(Point::new(center.x - arrow_dist, center.y));
                b.line_to(Point::new(
                    center.x - arrow_dist + arrow_len,
                    center.y + 5.0,
                ));
            }),
            arrow_stroke,
        );
        // Right arrow
        frame.stroke(
            &canvas::Path::new(|b| {
                b.move_to(Point::new(center.x + arrow_dist, center.y));
                b.line_to(Point::new(
                    center.x + arrow_dist - arrow_len,
                    center.y - 5.0,
                ));
                b.move_to(Point::new(center.x + arrow_dist, center.y));
                b.line_to(Point::new(
                    center.x + arrow_dist - arrow_len,
                    center.y + 5.0,
                ));
            }),
            arrow_stroke,
        );
        // Up arrow
        frame.stroke(
            &canvas::Path::new(|b| {
                b.move_to(Point::new(center.x, center.y - arrow_dist));
                b.line_to(Point::new(
                    center.x - 5.0,
                    center.y - arrow_dist + arrow_len,
                ));
                b.move_to(Point::new(center.x, center.y - arrow_dist));
                b.line_to(Point::new(
                    center.x + 5.0,
                    center.y - arrow_dist + arrow_len,
                ));
            }),
            arrow_stroke,
        );
        // Down arrow
        frame.stroke(
            &canvas::Path::new(|b| {
                b.move_to(Point::new(center.x, center.y + arrow_dist));
                b.line_to(Point::new(
                    center.x - 5.0,
                    center.y + arrow_dist - arrow_len,
                ));
                b.move_to(Point::new(center.x, center.y + arrow_dist));
                b.line_to(Point::new(
                    center.x + 5.0,
                    center.y + arrow_dist - arrow_len,
                ));
            }),
            arrow_stroke,
        );

        // Knob
        let knob_x = center.x + self.knob_x * radius * 0.85;
        let knob_y = center.y - self.knob_y * radius * 0.85;
        let knob_pos = Point::new(knob_x, knob_y);

        frame.fill(
            &canvas::Path::circle(knob_pos, 14.0),
            Color::from_rgba(0.3, 0.55, 1.0, 0.25),
        );
        frame.fill(
            &canvas::Path::circle(knob_pos, 10.0),
            Color::from_rgb(0.4, 0.65, 1.0),
        );
        frame.fill(
            &canvas::Path::circle(Point::new(knob_x - 2.0, knob_y - 2.0), 4.0),
            Color::from_rgba(0.7, 0.85, 1.0, 0.5),
        );

        // KB indicator
        if self.keyboard_focused {
            frame.fill_text(canvas::Text {
                content: "KB".to_string(),
                position: Point::new(bounds.width - 22.0, 4.0),
                color: Color::from_rgba(0.3, 1.0, 0.4, 0.6),
                size: 11.0.into(),
                ..canvas::Text::default()
            });
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &JoystickState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if cursor.position_in(bounds).is_some() {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

// ==================== App Logic ====================

fn boot() -> (MainPanel, Task<Message>) {
    let mut camera = Camera::wait_for("OBSBOT Tiny 2", Duration::from_secs(1));
    camera.set_verbose(debug_mode());

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
                    joystick_pos: (0.0, 0.0),
                    joystick_active: false,
                    keyboard_focused: false,
                    show_values: false,
                    current_pan: None,
                    current_tilt: None,
                    current_zoom: None,
                    preview_process: None,
                },
                Task::none(),
            )
        }
    };

    let axis_from_range = |r: tiny2::CtrlRange| -> PtzAxis {
        let step = if r.step > 0 {
            r.step
        } else {
            ((r.maximum - r.minimum) / 20).max(1)
        };
        PtzAxis {
            step: Some(step),
            min: r.minimum,
            max: r.maximum,
        }
    };
    let pan = camera
        .query_pan_range()
        .ok()
        .map(&axis_from_range)
        .unwrap_or_else(PtzAxis::unavailable);
    let tilt = camera
        .query_tilt_range()
        .ok()
        .map(&axis_from_range)
        .unwrap_or_else(PtzAxis::unavailable);
    let zoom = camera
        .query_zoom_range()
        .ok()
        .map(&axis_from_range)
        .unwrap_or_else(PtzAxis::unavailable);

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
            joystick_pos: (0.0, 0.0),
            joystick_active: false,
            keyboard_focused: false,
            show_values: false,
            current_pan: None,
            current_tilt: None,
            current_zoom: None,
            preview_process: None,
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
            state.joystick_active = false;
            state.joystick_pos = (0.0, 0.0);
        }
        Message::Tick => {
            if let Some(action) = state.held_action {
                state.execute_ptz(action);
            }
            if state.joystick_active {
                let (jx, jy) = state.joystick_pos;
                if let Some(pan_step) = state.pan.step {
                    let delta = (jx * pan_step as f32) as i32;
                    if delta != 0 {
                        state.execute_ptz(PtzAction::Pan(delta));
                    }
                }
                if let Some(tilt_step) = state.tilt.step {
                    let delta = (jy * tilt_step as f32) as i32;
                    if delta != 0 {
                        state.execute_ptz(PtzAction::Tilt(delta));
                    }
                }
            }
            if state.show_values {
                state.refresh_values();
            }
            // Check if preview process is still alive
            state.is_preview_running();
        }
        Message::JoystickDrag(x, y) => {
            state.joystick_pos = (x, y);
            state.joystick_active = true;
        }
        Message::JoystickRelease => {
            state.joystick_pos = (0.0, 0.0);
            state.joystick_active = false;
        }
        Message::JoystickDoubleClick => {
            state.keyboard_focused = !state.keyboard_focused;
        }
        Message::ScrollZoom(delta) => {
            if let Some(step) = state.zoom.step {
                let zoom_delta = if delta > 0.0 { -step } else { step };
                state.execute_ptz(PtzAction::Zoom(zoom_delta));
            }
        }
        Message::ToggleValuesPanel => {
            state.show_values = !state.show_values;
            if state.show_values {
                state.refresh_values();
            }
        }
        Message::KeyboardMove(pan_dir, tilt_dir) => {
            if state.keyboard_focused {
                if let Some(pan_step) = state.pan.step {
                    if pan_dir != 0 {
                        state.execute_ptz(PtzAction::Pan(pan_step * pan_dir));
                    }
                }
                if let Some(tilt_step) = state.tilt.step {
                    if tilt_dir != 0 {
                        state.execute_ptz(PtzAction::Tilt(tilt_step * tilt_dir));
                    }
                }
            }
        }
        Message::KeyboardZoom(dir) => {
            if state.keyboard_focused {
                if let Some(step) = state.zoom.step {
                    state.execute_ptz(PtzAction::Zoom(step * dir));
                }
            }
        }
        Message::KeyboardDeactivate => {
            state.keyboard_focused = false;
        }
        Message::TogglePreview => {
            if state.is_preview_running() {
                // Kill existing preview
                if let Some(ref mut child) = state.preview_process {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                state.preview_process = None;
            } else {
                // Launch new preview
                match find_obsbot_capture_device() {
                    Some(device) => match spawn_preview(&device) {
                        Ok(child) => {
                            state.preview_process = Some(child);
                        }
                        Err(msg) => {
                            state.error_message = Some(msg);
                        }
                    },
                    None => {
                        state.error_message =
                            Some("Could not find OBSBOT Tiny 2 capture device".to_string());
                    }
                }
            }
        }
    }
    Task::none()
}

// ==================== View ====================

fn tracking_tooltip(mode: AIMode) -> &'static str {
    match mode {
        AIMode::NoTracking => "Disable AI tracking - camera stays fixed",
        AIMode::NormalTracking => "Standard tracking - follows face and body",
        AIMode::UpperBody => "Frames upper body - ideal for presentations",
        AIMode::CloseUp => "Tight face framing - for headshots",
        AIMode::Headless => "Tracks body without needing face detection",
        AIMode::LowerBody => "Tracks lower body movements",
        AIMode::DeskMode => "Points down to capture desk and documents",
        AIMode::Whiteboard => "Frames and tracks a whiteboard",
        AIMode::Hand => "Follows hand gestures and movements",
        AIMode::Group => "Wide tracking for multiple people",
    }
}

fn val_str(v: Option<i32>) -> String {
    match v {
        Some(n) => format!("{}", n),
        None => "---".to_string(),
    }
}

fn styled_tooltip<'a>(
    content: impl Into<Element<'a, Message>>,
    tip: &'a str,
    position: tooltip::Position,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(tip).size(12))
            .padding([4, 8])
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.background.strong.color.into()),
                    text_color: Some(palette.background.strong.text),
                    border: Border {
                        radius: 6.0.into(),
                        width: 1.0,
                        color: palette.background.weak.color,
                    },
                    shadow: iced::Shadow {
                        color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 6.0,
                    },
                    ..Default::default()
                }
            }),
        position,
    )
    .gap(5)
    .into()
}

fn view(state: &MainPanel) -> Element<Message> {
    let track_btn = |label: &'static str, mode: AIMode| -> Element<Message> {
        let style = if state.tracking == mode {
            button::primary
        } else {
            button::secondary
        };
        styled_tooltip(
            button(text(label).align_x(Alignment::Center))
                .on_press(Message::ChangeTracking(mode))
                .style(style)
                .width(Length::Fill),
            tracking_tooltip(mode),
            tooltip::Position::Bottom,
        )
    };

    let exposure_btn =
        |label: &'static str, mode: ExposureMode, tip: &'static str| -> Element<Message> {
            styled_tooltip(
                button(text(label).align_x(Alignment::Center))
                    .on_press(Message::ChangeExposure(mode))
                    .width(Length::Fill),
                tip,
                tooltip::Position::Bottom,
            )
        };

    let fov_btn =
        |label: &'static str, mode: FOVMode, tip: &'static str| -> Element<Message> {
            styled_tooltip(
                button(text(label).align_x(Alignment::Center))
                    .on_press(Message::ChangeFOV(mode))
                    .width(Length::Fill),
                tip,
                tooltip::Position::Bottom,
            )
        };

    // ---- Camera Preview button ----
    let preview_running = state.preview_process.is_some();
    let preview_label = if preview_running {
        "Close Preview"
    } else {
        "Open Preview"
    };
    let preview_style = if preview_running {
        button::danger
    } else {
        button::success
    };

    let mut c = column![
        // Preview button at the top
        styled_tooltip(
            button(text(preview_label).align_x(Alignment::Center))
                .on_press(Message::TogglePreview)
                .style(preview_style)
                .width(Length::Fill),
            "Opens camera feed in a separate window (mpv/ffplay/vlc)",
            tooltip::Position::Bottom,
        ),
        // Tracking modes
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
        // Exposure
        row![
            exposure_btn("Manual", ExposureMode::Manual, "Manual exposure control"),
            exposure_btn(
                "Face",
                ExposureMode::Face,
                "Auto-expose based on face brightness"
            ),
            exposure_btn(
                "Global",
                ExposureMode::Global,
                "Auto-expose for the whole frame"
            ),
        ]
        .spacing(10),
        // FOV
        row![
            fov_btn(
                "FOV 86\u{b0}",
                FOVMode::Wide,
                "Wide - maximum field of view (86\u{b0})"
            ),
            fov_btn(
                "FOV 78\u{b0}",
                FOVMode::Normal,
                "Normal - standard field of view (78\u{b0})"
            ),
            fov_btn(
                "FOV 65\u{b0}",
                FOVMode::Narrow,
                "Narrow - tight field of view (65\u{b0})"
            ),
        ]
        .spacing(10),
        // HDR
        toggler(state.hdr_on)
            .label("HDR")
            .on_toggle(Message::ChangeHDR),
    ]
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .spacing(10)
    .padding(10);

    // ---- Error message ----
    if let Some(ref err) = state.error_message {
        c = c.push(
            container(text(err).size(12))
                .padding(6)
                .width(Length::Fill)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Color::from_rgba(0.8, 0.2, 0.2, 0.3).into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }

    // ---- PTZ controls ----
    if state.pan.step.is_some() || state.tilt.step.is_some() || state.zoom.step.is_some() {
        let kb_hint = if state.keyboard_focused {
            "Pan / Tilt / Zoom  [KB ON - arrows move, +/- zoom, Esc exits]"
        } else {
            "Pan / Tilt / Zoom  [double-click for keyboard]"
        };
        c = c.push(
            text(kb_hint)
                .size(11)
                .align_x(Alignment::Center)
                .width(Length::Fill),
        );

        // Joystick
        if state.pan.step.is_some() || state.tilt.step.is_some() {
            let joystick = canvas(JoystickProgram {
                knob_x: state.joystick_pos.0,
                knob_y: state.joystick_pos.1,
                keyboard_focused: state.keyboard_focused,
            })
            .width(Length::Fixed(160.0))
            .height(Length::Fixed(160.0));

            c = c.push(
                container(joystick)
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
            );
        }

        // Zoom buttons
        if let Some(step) = state.zoom.step {
            let ptz_btn = |label: &'static str| {
                container(text(label).align_x(Alignment::Center))
                    .padding([6, 16])
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

            let zoom_row = row![
                mouse_area(ptz_btn("\u{1f50d}\u{2796}"))
                    .on_press(Message::StartMove(PtzAction::Zoom(-step)))
                    .on_release(Message::StopMove),
                styled_tooltip(
                    text(" Scroll to zoom ").size(11),
                    "Use mouse scroll wheel to zoom in/out",
                    tooltip::Position::Bottom,
                ),
                mouse_area(ptz_btn("\u{1f50d}\u{2795}"))
                    .on_press(Message::StartMove(PtzAction::Zoom(step)))
                    .on_release(Message::StopMove),
            ]
            .spacing(10)
            .align_y(Alignment::Center);

            c = c.push(
                container(zoom_row)
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
            );
        }
    } else {
        c = c.push(text("PTZ controls not available for this device"));
    }

    // ---- Values panel toggle ----
    c = c.push(
        button(
            text(if state.show_values {
                "Hide Values"
            } else {
                "Show Values"
            })
            .size(12)
            .align_x(Alignment::Center),
        )
        .on_press(Message::ToggleValuesPanel)
        .style(button::secondary)
        .width(Length::Fill),
    );

    if state.show_values {
        let values_panel = container(
            column![
                text("Live PTZ Values").size(12),
                row![
                    text(format!("Pan: {}", val_str(state.current_pan))).size(12),
                    text(format!("Tilt: {}", val_str(state.current_tilt))).size(12),
                    text(format!("Zoom: {}", val_str(state.current_zoom))).size(12),
                ]
                .spacing(15),
                row![
                    text(format!("Pan[{}..{}]", state.pan.min, state.pan.max)).size(10),
                    text(format!("Tilt[{}..{}]", state.tilt.min, state.tilt.max)).size(10),
                    text(format!("Zoom[{}..{}]", state.zoom.min, state.zoom.max)).size(10),
                ]
                .spacing(15),
                text(format!(
                    "Joystick: ({:.2}, {:.2})  Active: {}  KB: {}",
                    state.joystick_pos.0,
                    state.joystick_pos.1,
                    state.joystick_active,
                    state.keyboard_focused,
                ))
                .size(11),
                text(format!(
                    "Tracking: {:?}  HDR: {}  Preview: {}",
                    state.tracking,
                    state.hdr_on,
                    state.preview_process.is_some()
                ))
                .size(11),
            ]
            .spacing(4),
        )
        .padding(8)
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: Border {
                    radius: 6.0.into(),
                    width: 1.0,
                    color: palette.background.strong.color,
                },
                ..Default::default()
            }
        });
        c = c.push(values_panel);
    }

    // ---- Debug hex commands (--debug) ----
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

    scrollable(c).into()
}

// ==================== Subscriptions ====================

fn handle_global_events(
    event: iced::Event,
    _status: event::Status,
    _id: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::StopMove)
        }
        iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
            let y = match delta {
                mouse::ScrollDelta::Lines { y, .. } => y,
                mouse::ScrollDelta::Pixels { y, .. } => y / 50.0,
            };
            if y.abs() > 0.1 {
                Some(Message::ScrollZoom(y))
            } else {
                None
            }
        }
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
                Some(Message::KeyboardMove(-1, 0))
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
                Some(Message::KeyboardMove(1, 0))
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                Some(Message::KeyboardMove(0, 1))
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                Some(Message::KeyboardMove(0, -1))
            }
            keyboard::Key::Character(ref ch) if ch.as_str() == "+" || ch.as_str() == "=" => {
                Some(Message::KeyboardZoom(1))
            }
            keyboard::Key::Character(ref ch) if ch.as_str() == "-" => {
                Some(Message::KeyboardZoom(-1))
            }
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                Some(Message::KeyboardDeactivate)
            }
            _ => None,
        },
        _ => None,
    }
}

fn subscription(state: &MainPanel) -> Subscription<Message> {
    let needs_tick =
        state.held_action.is_some() || state.joystick_active || state.show_values;
    let tick = if needs_tick {
        time::every(Duration::from_millis(100)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    };
    Subscription::batch(vec![tick, event::listen_with(handle_global_events)])
}

// ==================== Main ====================

fn main() -> iced::Result {
    // SAFETY: written once here before any other threads start.
    unsafe {
        DEBUG = std::env::args().any(|a| a == "--debug");
    }

    let window_height = if debug_mode() { 800.0 } else { 700.0 };

    iced::application(boot, update, view)
        .subscription(subscription)
        .window_size((420.0, window_height))
        .run()
}
