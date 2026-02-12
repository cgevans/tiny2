use rosc::{OscMessage, OscType};
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
            let (amt, _src) = socket.recv_from(&mut buf)?;

            let (_, packet) = rosc::decoder::decode_udp(&buf[..amt])?;

            match packet {
                rosc::OscPacket::Message(m) => self.handle_message(m)?,
                rosc::OscPacket::Bundle(_) => todo!(),
            }
        }
    }

    pub fn handle_message(&self, msg: OscMessage) -> Result<(), Error> {
        match msg.addr.as_str() {
            "/OBSBOT/WebCam/Tiny/SetAiMode" => {
                let mode = match msg.args[1] {
                    OscType::Int(x) => x,
                    _ => 0,
                };
                let camera = match msg.args[0] {
                    OscType::Int(x) => x as usize,
                    _ => 0,
                };
                self.cameras[camera].set_ai_mode(mode.try_into()?)
            }
            // PTZ absolute: /pan <camera> <value>, /tilt <camera> <value>, /zoom <camera> <value>
            "/pan" => {
                let camera = Self::get_camera_index(&msg, 0);
                let value = Self::get_int_arg(&msg, 1);
                self.cameras[camera].set_pan(value)
            }
            "/tilt" => {
                let camera = Self::get_camera_index(&msg, 0);
                let value = Self::get_int_arg(&msg, 1);
                self.cameras[camera].set_tilt(value)
            }
            "/zoom" => {
                let camera = Self::get_camera_index(&msg, 0);
                let value = Self::get_int_arg(&msg, 1);
                self.cameras[camera].set_zoom(value)
            }
            // PTZ relative: /pan/relative <camera> <delta>, etc.
            "/pan/relative" => {
                let camera = Self::get_camera_index(&msg, 0);
                let delta = Self::get_int_arg(&msg, 1);
                self.cameras[camera].pan_relative(delta)
            }
            "/tilt/relative" => {
                let camera = Self::get_camera_index(&msg, 0);
                let delta = Self::get_int_arg(&msg, 1);
                self.cameras[camera].tilt_relative(delta)
            }
            "/zoom/relative" => {
                let camera = Self::get_camera_index(&msg, 0);
                let delta = Self::get_int_arg(&msg, 1);
                self.cameras[camera].zoom_relative(delta)
            }
            _ => {
                println!("{:?}", msg);
                Ok(())
            }
        }
    }

    fn get_camera_index(msg: &OscMessage, arg_idx: usize) -> usize {
        match msg.args.get(arg_idx) {
            Some(OscType::Int(x)) => *x as usize,
            _ => 0,
        }
    }

    fn get_int_arg(msg: &OscMessage, arg_idx: usize) -> i32 {
        match msg.args.get(arg_idx) {
            Some(OscType::Int(x)) => *x,
            _ => 0,
        }
    }
}

use clap::Parser;
#[derive(Parser, Debug)]
#[command(author, version, about, long_about=None)]
struct Args {
    #[arg(short, long, default_value = "127.0.0.1:9000")]
    address: String,
}

fn main() {
    let args = Args::parse();

    let server = OBSBotOSCServer {
        addr: args.address,
        cameras: vec![Camera::wait_for("OBSBOT", Duration::from_secs(1))],
    };

    if let Err(err) = server.run_server() {
        eprintln!("Error: {}", err);
    }
}
