// SPDX-License-Identifier: EUPL-1.2
// Decode AI_GET_GIM_STATE by reading it at known poses.
use std::{thread, time::Duration};
fn show(tag: &str, cam: &tiny2::Camera) {
    match cam.gimbal_state_raw() {
        Ok(raw) => {
            let f32s: Vec<String> = raw.chunks(4).map(|c| {
                if c.len() == 4 { format!("{:.2}", f32::from_le_bytes([c[0],c[1],c[2],c[3]])) } else { "?".into() }
            }).collect();
            let i16s: Vec<String> = raw.chunks(2).map(|c| {
                if c.len()==2 { format!("{}", i16::from_le_bytes([c[0],c[1]])) } else { "?".into() }
            }).collect();
            println!("{tag}: len={} hex={}", raw.len(), hex::encode(&raw));
            println!("  as f32: [{}]", f32s.join(", "));
            println!("  as i16: [{}]", i16s.join(", "));
        }
        Err(e) => println!("{tag}: state read failed: {e}"),
    }
}
fn main() {
    let cam = tiny2::Camera::new("OBSBOT Tiny 2").unwrap();
    cam.wake_and_wait(Duration::from_secs(3)).unwrap();
    match cam.preset_list() {
        Ok(l) => println!("presets: {:?}", l),
        Err(e) => println!("preset list failed: {e}"),
    }
    show("state@current", &cam);
    cam.gimbal_move_to(15.0, 0.0).unwrap();
    thread::sleep(Duration::from_millis(2500));
    show("state@yaw15", &cam);
    cam.gimbal_move_to(-20.0, -10.0).unwrap();
    thread::sleep(Duration::from_millis(2500));
    show("state@yaw-20,pitch-10", &cam);
    cam.gimbal_recenter().unwrap();
    thread::sleep(Duration::from_millis(2500));
    show("state@recentered", &cam);
}
