// SPDX-License-Identifier: EUPL-1.2
// Round-trip verification of config commands via status read-back.
use std::{thread, time::Duration};
use tiny2::{AIMode, Camera};

fn status_bytes(cam: &Camera) -> [u8; 60] { cam.read_status_raw().unwrap() }

fn main() {
    let cam = Camera::new("OBSBOT Tiny 2").unwrap();
    cam.wake_and_wait(Duration::from_secs(3)).unwrap();
    let s = |cam: &Camera| { thread::sleep(Duration::from_millis(1200)); status_bytes(cam) };

    // zoom with speed: ratio 150 => zoom_pct 50
    cam.set_zoom_with_speed(150, 10).unwrap();
    let b = s(&cam);
    println!("zoom 1.5x: zoom_pct={} (expect ~50)", u16::from_le_bytes([b[4], b[5]]));
    cam.set_zoom_with_speed(100, 10).unwrap();
    let b = s(&cam);
    println!("zoom 1.0x: zoom_pct={} (expect 0)", u16::from_le_bytes([b[4], b[5]]));

    // track speed
    cam.set_track_speed_sport(true).unwrap();
    let b = s(&cam);
    println!("sport: byte24={} (expect 2)", b[0x24]);
    cam.set_track_speed_sport(false).unwrap();
    let b = s(&cam);
    println!("standard: byte24={} (expect 0)", b[0x24]);

    // suspend time
    cam.set_suspend_time(300).unwrap();
    let b = s(&cam);
    println!("suspend 300: bytes0a={} (expect 300)", i16::from_le_bytes([b[0xa], b[0xb]]));
    cam.set_suspend_time(120).unwrap();
    let b = s(&cam);
    println!("suspend 120: bytes0a={} (expect 120)", i16::from_le_bytes([b[0xa], b[0xb]]));

    // mic during sleep
    let orig = status_bytes(&cam)[0x10];
    cam.set_mic_during_sleep(orig == 0).unwrap();
    let b = s(&cam);
    println!("mic-sleep toggled: byte10={} (was {})", b[0x10], orig);
    cam.set_mic_during_sleep(orig != 0).unwrap();
    let b = s(&cam);
    println!("mic-sleep restored: byte10={} (expect {})", b[0x10], orig);

    // boot AI mode
    let orig_boot = status_bytes(&cam)[0x20];
    cam.set_boot_ai_mode(AIMode::NormalTracking).unwrap();
    let b = s(&cam);
    println!("boot normal-tracking: byte20=0x{:02x} (was 0x{:02x})", b[0x20], orig_boot);
    cam.set_boot_ai_mode(AIMode::NoTracking).unwrap();
    let b = s(&cam);
    println!("boot restored: byte20=0x{:02x} (expect 0x00)", b[0x20]);

    // boot pose: read, write the same value back, read again
    match cam.boot_pose_get() {
        Ok(pose) => {
            println!("boot pose: yaw {:.2} pitch {:.2} roll {:.2} zoom {:.2}", pose.yaw, pose.pitch, pose.roll, pose.zoom);
            cam.boot_pose_set(pose).unwrap();
            let again = cam.boot_pose_get().unwrap();
            println!("boot pose after same-value set: yaw {:.2} pitch {:.2} (expect unchanged) match={}",
                again.yaw, again.pitch, again == pose);
        }
        Err(e) => println!("boot pose get failed: {e}"),
    }
}
