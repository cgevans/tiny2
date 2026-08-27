// SPDX-License-Identifier: EUPL-1.2
// Hardware probe for Camera::gimbal_speed sign conventions; moves the
// gimbal briefly. Usage: gimbal_speed_probe <yaw_dps> <pitch_dps> <ms>
use std::time::Duration;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let yaw: f32 = args[1].parse().unwrap();
    let pitch: f32 = args[2].parse().unwrap();
    let ms: u64 = args[3].parse().unwrap();
    let cam = tiny2::Camera::new("OBSBOT Tiny 2").unwrap();
    cam.gimbal_speed(yaw, pitch).unwrap();
    std::thread::sleep(Duration::from_millis(ms));
    cam.gimbal_stop().unwrap();
    println!("done: yaw {} pitch {} for {} ms", yaw, pitch, ms);
}
