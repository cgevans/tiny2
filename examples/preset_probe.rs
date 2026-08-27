// SPDX-License-Identifier: EUPL-1.2
// Read-only preset listing plus a recall/recenter round trip.
use std::{thread, time::Duration};
fn main() {
    let cam = tiny2::Camera::new("OBSBOT Tiny 2").unwrap();
    cam.wake_and_wait(Duration::from_secs(3)).unwrap();
    for p in cam.preset_list().unwrap() {
        println!("slot {}: {:?} yaw {:.2} pitch {:.2} zoom {:.2}",
            p.slot, p.name, p.pose.yaw, p.pose.pitch, p.pose.zoom);
    }
    println!("recalling preset 1...");
    cam.preset_recall(1).unwrap();
    thread::sleep(Duration::from_millis(3000));
    println!("recentering...");
    cam.gimbal_recenter().unwrap();
}
