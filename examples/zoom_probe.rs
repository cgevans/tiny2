// SPDX-License-Identifier: EUPL-1.2
use std::{thread, time::Duration};
fn main() {
    let cam = tiny2::Camera::new("OBSBOT Tiny 2").unwrap();
    cam.wake_and_wait(Duration::from_secs(3)).unwrap();
    let zoom = |cam: &tiny2::Camera| {
        let b = cam.read_status_raw().unwrap();
        u16::from_le_bytes([b[4], b[5]])
    };
    for speed in [1u32, 5, 10, 20] {
        cam.set_zoom_with_speed(150, speed).unwrap();
        thread::sleep(Duration::from_millis(2500));
        let z = zoom(&cam);
        cam.set_zoom_with_speed(100, 10).unwrap();
        thread::sleep(Duration::from_millis(2500));
        println!("speed {}: reached {}% (restored to {}%)", speed, z, zoom(&cam));
    }
    // stop test at a valid speed
    cam.set_zoom_with_speed(200, 2).unwrap();
    thread::sleep(Duration::from_millis(1200));
    cam.zoom_stop().unwrap();
    thread::sleep(Duration::from_millis(800));
    let held = zoom(&cam);
    thread::sleep(Duration::from_millis(1500));
    println!("zoom_stop: held at {}% then {}% (stop works if equal and 0<x<100)", held, zoom(&cam));
    cam.set_zoom_with_speed(100, 10).unwrap();
    thread::sleep(Duration::from_millis(2000));
    println!("final: {}%", zoom(&cam));
}
