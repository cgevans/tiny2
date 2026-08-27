// SPDX-License-Identifier: EUPL-1.2
//! Exercises the preset write paths against real hardware: update a pose,
//! rename, and recall. Restores whatever it found when it started.
//!
//! Deliberately does NOT delete: deletion is irreversible from Linux
//! because nothing decoded can create a preset in a free slot (see the
//! preset module docs).
use std::{thread, time::Duration};
use tiny2::{preset::Pose, Camera};

fn list(cam: &Camera, tag: &str) -> Vec<tiny2::preset::Preset> {
    thread::sleep(Duration::from_millis(800));
    let l = cam.preset_list().unwrap_or_default();
    println!("--- {} ({} presets)", tag, l.len());
    for p in &l {
        println!(
            "    slot {}: {:?} yaw {:.2} pitch {:.2} zoom {:.2}",
            p.slot, p.name, p.pose.yaw, p.pose.pitch, p.pose.zoom
        );
    }
    l
}

fn main() {
    let cam = Camera::new("OBSBOT Tiny 2").unwrap();
    cam.wake_and_wait(Duration::from_secs(3)).unwrap();

    let before = list(&cam, "initial");
    let Some(original) = before.first().cloned() else {
        println!("no presets stored; nothing to exercise");
        return;
    };
    let slot = original.slot;

    // Update with an asymmetric pose: yaw and pitch differ in magnitude
    // and sign, so a swapped field order cannot pass unnoticed.
    println!("\n[update] slot {} = yaw 30.0 pitch -5.0 zoom 1.5", slot);
    cam.preset_update(slot, Pose { yaw: 30.0, pitch: -5.0, roll: 0.0, zoom: 1.5 })
        .unwrap();
    let l = list(&cam, "after update");
    if let Some(p) = l.iter().find(|p| p.slot == slot) {
        let ok = (p.pose.yaw - 30.0).abs() < 0.5 && (p.pose.pitch + 5.0).abs() < 0.5;
        println!("    => update {}", if ok { "OK" } else { "MISMATCH" });
    }

    println!("\n[rename] slot {} -> \"ProbeName\"", slot);
    cam.preset_set_name(slot, "ProbeName").unwrap();
    let l = list(&cam, "after rename");
    let got = l.iter().find(|p| p.slot == slot).map(|p| p.name.clone());
    println!("    => rename {}", if got.as_deref() == Some("ProbeName") { "OK" } else { "MISMATCH" });

    println!("\n[recall] moving away, then recalling slot {}", slot);
    cam.gimbal_move_to(-30.0, 10.0).unwrap();
    thread::sleep(Duration::from_millis(2500));
    cam.preset_recall(slot).unwrap();
    thread::sleep(Duration::from_millis(3000));

    println!("\n[restore] slot {} -> {:?}", slot, original.name);
    cam.preset_update(slot, original.pose).unwrap();
    thread::sleep(Duration::from_millis(600));
    if !original.name.is_empty() {
        cam.preset_set_name(slot, &original.name).unwrap();
    }
    list(&cam, "final");
    cam.gimbal_recenter().unwrap();
}
