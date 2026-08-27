// SPDX-License-Identifier: EUPL-1.2

//! obsbot-v4l2-mapper: install dynamic V4L2 control mappings for the Tiny
//! 2, so ordinary V4L2 applications can use its vendor controls. The
//! driver-behaviour notes in this file are LLM-drafted (Claude).

use clap::{Parser, Subcommand};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tiny2::mapping::{self, ApplyOutcome, MenuMapping};
use tiny2::{AIMode, Camera, Error, FOVMode};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Install dynamic V4L2 control mappings for the OBSBot Tiny 2, \
             so any V4L2 application can use its vendor controls"
)]
struct Args {
    /// Device path, /dev name, or substring of the card/bus name.
    #[arg(short, long, default_value = "OBSBOT Tiny 2", global = true)]
    device: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum PresetCmd {
    /// Recall a preset by slot (1-3).
    Recall { slot: u8 },
    /// Overwrite an existing preset's pose (yaw/pitch in degrees, zoom
    /// ratio 1.0-2.0). Cannot create a preset in an empty slot.
    Update {
        slot: u8,
        yaw: f32,
        pitch: f32,
        #[arg(default_value_t = 1.0)]
        zoom: f32,
    },
    /// Delete a preset; irreversible, this tool cannot create one.
    Delete { slot: u8 },
    /// Rename a preset.
    Name { slot: u8, name: String },
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// Auto-sleep delay in seconds.
    SleepTimer { secs: u16 },
    /// Keep the microphone live while the camera sleeps.
    MicInSleep { on_off: String },
    /// AI tracking mode engaged at power-on (e.g. none, normal, upper-body).
    BootTracking { mode: String },
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Install all OBSBot control mappings on the device (idempotent).
    Apply,
    /// List the device's V4L2 controls, including mapped OBSBot ones.
    List,
    /// Show the camera's current status (tracking mode, HDR, PTZ).
    Status {
        /// Also hex-dump the raw 60-byte status block.
        #[arg(long)]
        raw: bool,
    },
    /// Set a mapped control, e.g. `set tracking upper-body`, `set hdr on`.
    /// Installs the mapping first if needed.
    Set {
        /// Control key: tracking, hdr, fov, face-ae, mirror, led (solid or
    /// blinking).
        control: String,
        /// Menu entry label (fuzzy) or index; omit to list choices.
        value: Option<String>,
    },
    /// Move the gimbal to absolute angles in degrees (positive yaw
    /// right, positive pitch down).
    Goto { yaw: f32, pitch: f32 },
    /// Recenter the gimbal.
    Recenter,
    /// List gimbal presets, or manage them: `preset`, `preset recall 2`,
    /// `preset update ...`, `preset name ...`, `preset delete ...`.
    Preset {
        #[command(subcommand)]
        action: Option<PresetCmd>,
    },
    /// Zoom to a ratio (1.0-2.0 or 100-200) at a speed (1-10).
    Zoom {
        ratio: f32,
        #[arg(long, default_value_t = 5)]
        speed: u32,
    },
    /// Tracking responsiveness: standard or sport.
    TrackSpeed { value: String },
    /// Camera configuration: sleep-timer, mic-in-sleep, boot-tracking.
    Config {
        #[command(subcommand)]
        setting: ConfigCmd,
    },
    /// Wake the camera from auto-sleep.
    Wake,
    /// Put the camera to sleep (gimbal parks, video blanks).
    Sleep,
    /// Print (or install) a udev rule that re-applies the mappings
    /// automatically whenever the camera is plugged in.
    Udev {
        /// Write the rule to /etc/udev/rules.d instead of printing it.
        #[arg(long)]
        install: bool,
    },
}

fn find_mapping(key: &str) -> Option<&'static MenuMapping> {
    let lower = key.to_ascii_lowercase();
    mapping::ALL
        .iter()
        .copied()
        .find(|m| m.key == lower || lower == "ai" && m.key == "tracking")
}

fn open(device: &str) -> Result<Camera, Error> {
    let cam = Camera::new(device)?;
    if let Ok(info) = cam.device_info() {
        eprintln!("Using {} ({})", info.card, info.bus_info);
    }
    Ok(cam)
}

fn apply(cam: &Camera) -> bool {
    let mut ok = true;
    for (m, outcome) in cam.apply_all_mappings() {
        match outcome {
            Ok(ApplyOutcome::Installed) => {
                println!("{:26} (0x{:08x}): installed", m.name, m.cid)
            }
            Ok(ApplyOutcome::AlreadyMapped) => {
                println!("{:26} (0x{:08x}): already mapped", m.name, m.cid)
            }
            Err(e) => {
                ok = false;
                eprintln!("{:26} (0x{:08x}): FAILED: {}", m.name, m.cid, e)
            }
        }
    }
    ok
}

fn list(cam: &Camera) -> Result<(), Error> {
    println!("{:<10} {:<32} {:>6} range", "id", "name", "type");
    let mut id = 0;
    while let Some(q) = cam.next_ctrl(id)? {
        id = q.id;
        let obsbot = mapping::ALL.iter().any(|m| m.cid == q.id);
        let type_name = match q.ctrl_type {
            1 => "int",
            2 => "bool",
            3 => "menu",
            4 => "button",
            6 => "class",
            9 => "imenu",
            t => {
                println!(
                    "0x{:08x} {:<32} {:>6}",
                    q.id,
                    q.name,
                    format!("({})", t)
                );
                continue;
            }
        };
        if q.ctrl_type == 6 {
            println!("--- {} ---", q.name);
            continue;
        }
        println!(
            "0x{:08x} {:<32} {:>6} {}..{} step {} default {}{}",
            q.id,
            q.name,
            type_name,
            q.minimum,
            q.maximum,
            q.step,
            q.default_value,
            if obsbot { "  [obsbot mapping]" } else { "" },
        );
        if q.ctrl_type == 3 || q.ctrl_type == 9 {
            for index in q.minimum..=q.maximum {
                if let Some(name) = cam.query_menu(q.id, index as u32)? {
                    println!("             {}: {}", index, name);
                }
            }
        }
    }
    Ok(())
}

fn status(cam: &Camera, raw: bool) -> Result<(), Error> {
    let block = cam.read_status_raw()?;
    match tiny2::CameraStatus::decode(&block) {
        Ok(s) => {
            match s.ai_mode {
                Some(mode) => println!("AI tracking: {}", mode),
                None => println!(
                    "AI tracking: unknown/switching (code {}, {})",
                    s.ai_code.0, s.ai_code.1
                ),
            }
            println!("HDR:         {}", if s.hdr_on { "on" } else { "off" });
            println!(
                "FOV:         {}",
                match s.fov {
                    Some(f) => format!("{} deg", f.degrees()),
                    None => "custom zoom override".to_string(),
                }
            );
            println!("Face AE:     {}", if s.face_ae { "on" } else { "off" });
            if s.mirrored {
                println!("Mirror:      on");
            }
            println!("Zoom:        {}%", s.zoom_pct);
            println!(
                "Track speed: {}",
                match s.tracker_speed {
                    0 => "standard".to_string(),
                    2 => "sport".to_string(),
                    v => format!("{}", v),
                }
            );
            if s.fps > 0 {
                println!("Stream FPS:  {}", s.fps);
            }
            if s.sleeping {
                println!(
                    "Sleeping:    yes (some commands are ignored until the \
                     camera wakes, e.g. when streaming starts)"
                );
            }
        }
        Err(e) => println!("status: {}", e),
    }
    if let (Ok(p), Ok(t), Ok(z)) = (cam.get_pan(), cam.get_tilt(), cam.get_zoom()) {
        println!(
            "PTZ:         pan {:.1} deg, tilt {:.1} deg, zoom {}",
            p as f64 / 3600.0,
            t as f64 / 3600.0,
            z
        );
    }
    match cam.get_exposure() {
        Ok((mode, value)) => println!(
            "Exposure:    {}",
            match mode {
                1 => format!("manual, shutter {:.1} ms", value as f64 / 10.0),
                2 => format!("auto, AE level {}", value),
                m => format!("mode {} value {}", m, value),
            }
        ),
        Err(e) => println!("Exposure:    unavailable ({})", e),
    }
    if let Ok(sn) = cam.serial_number() {
        println!("Serial:      {}", sn);
    }
    if let Ok(t) = cam.get_suspend_time() {
        println!("Auto-sleep:  {} s", t);
    }
    if raw {
        for row in block.chunks(16) {
            println!("  {}", hex::encode(row));
        }
    }
    Ok(())
}

fn set(cam: &Camera, control: &str, value: Option<&str>) -> Result<bool, Error> {
    let Some(m) = find_mapping(control) else {
        eprintln!(
            "unknown control '{}'; known controls: {}",
            control,
            mapping::ALL
                .iter()
                .map(|m| m.key)
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(false);
    };
    let Some(value) = value else {
        println!("{} ({}):", m.name, m.key);
        for (i, (label, _)) in m.entries.iter().enumerate() {
            println!("  {}: {}", i, label);
        }
        return Ok(true);
    };
    let Some((index, label)) = m.find_entry(value) else {
        eprintln!(
            "unknown value '{}' for {}; choices: {}",
            value,
            m.key,
            m.entries
                .iter()
                .map(|(l, _)| *l)
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(false);
    };
    // Most commands are ignored while the camera sleeps, so wake it first.
    // (Only now: the argument checks above must not wake the device just
    // to print text.)
    if let Err(e) = cam.wake_and_wait(Duration::from_secs(3)) {
        eprintln!("warning: could not wake camera ({}); trying anyway", e);
    }
    cam.apply_mapping(m)?;
    cam.set_ctrl(m.cid, index)?;
    println!("{} -> {}", m.name, label);

    // Confirm through the status block; changes can lag a couple seconds.
    match wait_for_status(cam, m.key, index) {
        Some(true) => println!("confirmed via camera status"),
        Some(false) => {
            eprintln!(
                "warning: camera status does not show the change. If this \
                 mapping was installed by an older version of this tool, the \
                 driver still has the old table (mappings persist until the \
                 camera is unplugged); replug the camera and re-apply."
            );
            return Ok(false);
        }
        None => {}
    }
    Ok(true)
}

/// Poll the status block until it shows menu entry `index` of the `key`
/// control, for up to ~4 s. None if the control has no status feedback.
fn wait_for_status(cam: &Camera, key: &str, index: i32) -> Option<bool> {
    let expect = |s: &tiny2::CameraStatus| -> Option<bool> {
        match key {
            "tracking" => Some(s.ai_mode == Some(AIMode::ALL[index as usize])),
            "hdr" => Some(s.hdr_on == (index == 1)),
            "face-ae" => Some(s.face_ae == (index == 1)),
            "fov" => Some(s.fov == FOVMode::from_status(index as u8)),
            "mirror" => Some(s.mirrored == (index == 1)),
            _ => None,
        }
    };
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Ok(s) = cam.get_status() {
            match expect(&s) {
                None => return None,
                Some(true) => return Some(true),
                Some(false) if Instant::now() >= deadline => return Some(false),
                Some(false) => {}
            }
        } else if Instant::now() >= deadline {
            return Some(false);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/71-obsbot-v4l2-mapper.rules";

fn udev_rule() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "/usr/local/bin/obsbot-v4l2-mapper".to_string());
    // ATTR{index}=="0" limits the rule to the default video node, so it
    // fires once per plug instead of once per node.
    format!(
        r#"# Re-apply OBSBot Tiny 2 dynamic V4L2 control mappings on hotplug.
# Installed by obsbot-v4l2-mapper.
ACTION=="add", SUBSYSTEM=="video4linux", ATTR{{index}}=="0", ATTRS{{idVendor}}=="3564", ATTRS{{idProduct}}=="fef8", RUN+="{} --device $env{{DEVNAME}} apply"
"#,
        exe
    )
}

fn udev(install: bool) -> Result<bool, Error> {
    let rule = udev_rule();
    if let Some(path) = rule.split("RUN+=\"").nth(1).and_then(|r| r.split(' ').next()) {
        const SYSTEM_DIRS: [&str; 7] = [
            "/usr/bin/", "/usr/local/bin/", "/usr/sbin/", "/bin/", "/sbin/", "/usr/lib/", "/opt/",
        ];
        if !SYSTEM_DIRS.iter().any(|d| path.starts_with(d)) {
            eprintln!(
                "warning: this rule embeds the current binary path ({}), which \
                 systemd-udevd may be unable to execute (SELinux confines it to \
                 system locations). Install the binary to e.g. /usr/local/bin \
                 and regenerate the rule from there.",
                path
            );
        }
    }
    if !install {
        print!("{}", rule);
        println!("# To install: obsbot-v4l2-mapper udev --install   (as root)");
        println!("# or save the rule above to {}", UDEV_RULE_PATH);
        return Ok(true);
    }
    match std::fs::write(UDEV_RULE_PATH, &rule) {
        Ok(()) => {
            println!("Wrote {}", UDEV_RULE_PATH);
            println!("Reload with: udevadm control --reload");
            println!("The rule runs the next time the camera is (re)plugged.");
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "Permission denied writing {}: run as root, or print the \
                 rule with `obsbot-v4l2-mapper udev` and install it yourself.",
                UDEV_RULE_PATH
            );
            Ok(false)
        }
        Err(e) => Err(e.into()),
    }
}

fn run(args: &Args) -> Result<bool, Error> {
    match &args.command {
        Command::Udev { install } => udev(*install),
        Command::Apply => Ok(apply(&open(&args.device)?)),
        Command::List => {
            list(&open(&args.device)?)?;
            Ok(true)
        }
        Command::Status { raw } => {
            status(&open(&args.device)?, *raw)?;
            Ok(true)
        }
        Command::Set { control, value } => set(&open(&args.device)?, control, value.as_deref()),
        Command::Goto { yaw, pitch } => {
            let cam = open(&args.device)?;
            cam.wake_and_wait(Duration::from_secs(3))?;
            cam.gimbal_move_to(*yaw, *pitch)?;
            println!("moving to yaw {} pitch {}", yaw, pitch);
            Ok(true)
        }
        Command::Recenter => {
            let cam = open(&args.device)?;
            cam.wake_and_wait(Duration::from_secs(3))?;
            cam.gimbal_recenter()?;
            println!("recentering");
            Ok(true)
        }
        Command::Preset { action } => {
            let cam = open(&args.device)?;
            cam.wake_and_wait(Duration::from_secs(3))?;
            match action {
                None => {
                    let presets = cam.preset_list()?;
                    if presets.is_empty() {
                        println!("no presets stored");
                    }
                    for p in presets {
                        println!(
                            "slot {}: {:?}  yaw {:.1} pitch {:.1} zoom {:.2}",
                            p.slot, p.name, p.pose.yaw, p.pose.pitch, p.pose.zoom
                        );
                    }
                }
                Some(PresetCmd::Recall { slot }) => {
                    cam.preset_recall(*slot)?;
                    println!("recalling preset {}", slot);
                }
                Some(PresetCmd::Update { slot, yaw, pitch, zoom }) => {
                    cam.preset_update(
                        *slot,
                        tiny2::preset::Pose {
                            yaw: *yaw,
                            pitch: *pitch,
                            roll: 0.0,
                            zoom: *zoom,
                        },
                    )?;
                    println!("updated preset {}", slot);
                }
                Some(PresetCmd::Delete { slot }) => {
                    cam.preset_delete(*slot)?;
                    println!("deleted preset {}", slot);
                    eprintln!(
                        "warning: irreversible; this tool cannot create a preset in a free slot."
                    );
                }
                Some(PresetCmd::Name { slot, name }) => {
                    cam.preset_set_name(*slot, name)?;
                    println!("renamed preset {}", slot);
                }
            }
            Ok(true)
        }
        Command::Zoom { ratio, speed } => {
            let cam = open(&args.device)?;
            cam.wake_and_wait(Duration::from_secs(3))?;
            let x100 = if *ratio <= 2.0 { (*ratio * 100.0) as u32 } else { *ratio as u32 };
            if !(100..=200).contains(&x100) {
                eprintln!("zoom ratio must be 1.0-2.0 (or 100-200)");
                return Ok(false);
            }
            cam.set_zoom_with_speed(x100, *speed)?;
            println!("zooming to {:.2}x", x100 as f32 / 100.0);
            Ok(true)
        }
        Command::TrackSpeed { value } => {
            let cam = open(&args.device)?;
            let sport = match value.to_ascii_lowercase().as_str() {
                "sport" | "motion" | "fast" | "2" => true,
                "standard" | "normal" | "0" => false,
                other => {
                    eprintln!("unknown track speed '{}'; use standard or sport", other);
                    return Ok(false);
                }
            };
            cam.wake_and_wait(Duration::from_secs(3))?;
            cam.set_track_speed_sport(sport)?;
            println!("track speed: {}", if sport { "sport" } else { "standard" });
            Ok(true)
        }
        Command::Config { setting } => {
            let cam = open(&args.device)?;
            cam.wake_and_wait(Duration::from_secs(3))?;
            let on = |s: &str| matches!(s, "on" | "true" | "1" | "yes");
            match setting {
                ConfigCmd::SleepTimer { secs } => {
                    cam.set_suspend_time(*secs)?;
                    println!("auto-sleep delay: {} s", secs);
                }
                ConfigCmd::MicInSleep { on_off } => {
                    cam.set_mic_during_sleep(on(on_off))?;
                    println!("microphone during sleep: {}", on_off);
                }
                ConfigCmd::BootTracking { mode } => {
                    let Some((index, label)) = mapping::AI_TRACKING.find_entry(mode) else {
                        eprintln!("unknown tracking mode '{}'", mode);
                        return Ok(false);
                    };
                    cam.set_boot_ai_mode(AIMode::ALL[index as usize])?;
                    println!("boot tracking mode: {}", label);
                }
            }
            Ok(true)
        }
        Command::Wake => {
            let cam = open(&args.device)?;
            cam.wake_and_wait(Duration::from_secs(3))?;
            println!("awake");
            Ok(true)
        }
        Command::Sleep => {
            open(&args.device)?.sleep()?;
            println!("sleep requested");
            Ok(true)
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}
