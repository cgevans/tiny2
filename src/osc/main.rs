// SPDX-License-Identifier: EUPL-1.2

//! obsbot-osc-server: control OBSBot cameras over OSC/UDP.
//! The notes in this file are LLM-drafted (Claude).
//!
//! Addresses (camera index first, then value; both 0 if omitted):
//!
//! - `/OBSBOT/WebCam/Tiny/SetAiMode <cam> <mode>`: 0 none, 1 normal,
//!   2 upper body, 3 close-up, 4 headless, 5 lower body, 6 desk,
//!   7 whiteboard, 8 hand, 9 group
//! - `/pan`, `/tilt`, `/zoom` `<cam> <value>`: absolute PTZ
//! - `/pan/relative`, `/tilt/relative`, `/zoom/relative` `<cam> <delta>`
//! - `/hdr <cam> <0|1>`
//! - `/fov <cam> <0|1|2>`: 86°, 78°, 65°
//! - `/exposure <cam> <0|1|2>`: manual, global auto, face auto
//! - `/mirror <cam> <0|1>`
//! - `/led <cam> <0|1>`: LED solid / blinking
//! - `/wake <cam>`, `/sleep <cam>`
//! - `/recenter <cam>`
//! - `/preset <cam> <slot>`: recall gimbal preset 1-3
//! - `/track-speed <cam> <0|1>`: standard / sport
//! - `/zoom-ratio <cam> <100..200>`: zoom with speed (1.30x = 130)

use rosc::{OscMessage, OscPacket, OscType};
use std::net::UdpSocket;
use std::time::Duration;
use tiny2::{Camera, Error, OBSBotWebCam};

struct OBSBotOSCServer {
    addr: String,
    cameras: Vec<Camera>,
}

impl OBSBotOSCServer {
    pub fn run_server(&self) -> Result<(), Error> {
        let socket = UdpSocket::bind(&self.addr)?;

        let mut buf = [0; 1024];
        loop {
            let (amt, src) = match socket.recv_from(&mut buf) {
                Ok(r) => r,
                Err(e) => {
                    // Transient errors (e.g. ICMP port-unreachable) should
                    // not kill the server.
                    eprintln!("recv error: {}", e);
                    continue;
                }
            };

            let packet = match rosc::decoder::decode_udp(&buf[..amt]) {
                Ok((_, packet)) => packet,
                Err(e) => {
                    eprintln!("ignoring malformed packet from {}: {}", src, e);
                    continue;
                }
            };

            self.handle_packet(packet);
        }
    }

    fn handle_packet(&self, packet: OscPacket) {
        match packet {
            OscPacket::Message(m) => {
                let addr = m.addr.clone();
                if let Err(e) = self.handle_message(m) {
                    eprintln!("{}: {}", addr, e);
                }
            }
            OscPacket::Bundle(b) => {
                for inner in b.content {
                    self.handle_packet(inner);
                }
            }
        }
    }

    fn handle_message(&self, msg: OscMessage) -> Result<(), Error> {
        let camera = self.camera(&msg)?;
        let value = Self::get_int_arg(&msg, 1);

        match msg.addr.as_str() {
            "/OBSBOT/WebCam/Tiny/SetAiMode" | "/ai" => camera.set_ai_mode(value.try_into()?),
            "/pan" => camera.set_pan(value),
            "/tilt" => camera.set_tilt(value),
            "/zoom" => camera.set_zoom(value),
            "/pan/relative" => camera.pan_relative(value),
            "/tilt/relative" => camera.tilt_relative(value),
            "/zoom/relative" => camera.zoom_relative(value),
            "/hdr" => camera.set_hdr_mode(value != 0),
            "/fov" => camera.set_fov(value.try_into()?),
            "/exposure" => camera.set_exposure_mode(value.try_into()?),
            "/mirror" => camera.set_mirror(value != 0),
            "/led" => camera.set_led_blinking(value != 0),
            "/wake" => camera.wake_and_wait(Duration::from_secs(3)),
            "/sleep" => camera.sleep(),
            "/recenter" => camera.gimbal_recenter(),
            "/preset" => camera.preset_recall(u8::try_from(value).map_err(|_| {
                Error::UnsupportedIntValue("preset slot".to_string(), value)
            })?),
            "/track-speed" => camera.set_track_speed_sport(value != 0),
            "/zoom-ratio" => {
                if !(100..=200).contains(&value) {
                    return Err(Error::UnsupportedIntValue(
                        "zoom ratio x100".to_string(),
                        value,
                    ));
                }
                camera.set_zoom_with_speed(value as u32, 5)
            }
            other => {
                eprintln!("unknown address: {:?} {:?}", other, msg.args);
                Ok(())
            }
        }
    }

    fn camera(&self, msg: &OscMessage) -> Result<&Camera, Error> {
        let index = Self::get_int_arg(msg, 0);
        usize::try_from(index)
            .ok()
            .and_then(|i| self.cameras.get(i))
            .ok_or_else(|| Error::UnsupportedIntValue("camera index".to_string(), index))
    }

    fn get_int_arg(msg: &OscMessage, arg_idx: usize) -> i32 {
        match msg.args.get(arg_idx) {
            Some(OscType::Int(x)) => *x,
            // Saturate out-of-range values like the float casts, not wrap.
            Some(OscType::Long(x)) => (*x).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            Some(OscType::Float(x)) => *x as i32,
            Some(OscType::Double(x)) => *x as i32,
            _ => 0,
        }
    }
}

use clap::Parser;
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "OSC/UDP server for controlling OBSBot cameras"
)]
struct Args {
    #[arg(short, long, default_value = "127.0.0.1:9000")]
    address: String,

    /// Device path, /dev name, or substring of the card/bus name.
    #[arg(short, long, default_value = "OBSBOT")]
    device: String,
}

fn main() {
    let args = Args::parse();

    let server = OBSBotOSCServer {
        addr: args.address,
        cameras: vec![Camera::wait_for(&args.device, Duration::from_secs(1))],
    };

    if let Err(err) = server.run_server() {
        eprintln!("Error: {}", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(args: Vec<OscType>) -> OscMessage {
        OscMessage {
            addr: "/test".to_string(),
            args,
        }
    }

    #[test]
    fn int_arg_coercions() {
        let get = |args, i| OBSBotOSCServer::get_int_arg(&msg(args), i);
        assert_eq!(get(vec![OscType::Int(7)], 0), 7);
        assert_eq!(get(vec![OscType::Long(7)], 0), 7);
        assert_eq!(get(vec![OscType::Long(i64::MAX)], 0), i32::MAX);
        assert_eq!(get(vec![OscType::Long(i64::MIN)], 0), i32::MIN);
        assert_eq!(get(vec![OscType::Long(1 << 32)], 0), i32::MAX);
        assert_eq!(get(vec![OscType::Float(2.9)], 0), 2);
        assert_eq!(get(vec![OscType::Double(-3.5)], 0), -3);
        assert_eq!(get(vec![OscType::Double(4294967296.0)], 0), i32::MAX);
        // missing or non-numeric arguments default to 0
        assert_eq!(get(vec![], 0), 0);
        assert_eq!(get(vec![OscType::Int(1)], 1), 0);
        assert_eq!(get(vec![OscType::String("x".into())], 0), 0);
    }
}
