//! Prints the health of every physical disk, as the app's Disk health screen
//! shows it. S.M.A.R.T. data needs an administrator terminal:
//!
//! cargo run --release -p scanner-core --example health_report

use scanner_core::format_size;
use scanner_core::health::list_disks;

fn main() {
    println!("elevated: {}", scanner_core::is_elevated());
    for d in list_disks() {
        let kind = match d.rotational {
            Some(true) => "HDD",
            Some(false) => "SSD",
            None => "?",
        };
        println!(
            "\nPhysicalDrive{}: {} [{} {kind}] {}  serial {}  firmware {}",
            d.number,
            d.model,
            d.bus,
            format_size(d.size),
            d.serial,
            d.firmware
        );
        println!("  verdict: {:?} {:?}  smart error: {:?}", d.verdict, d.reasons, d.smart_error);
        let f = &d.facts;
        println!(
            "  temp {:?} C, power-on {:?} h, cycles {:?}, life {:?}%, written {}, read {}",
            f.temperature,
            f.power_on_hours,
            f.power_cycles,
            f.life_left,
            f.bytes_written.map_or("-".into(), format_size),
            f.bytes_read.map_or("-".into(), format_size),
        );
        for a in &d.attributes {
            println!(
                "  {:02X} {:<32} {:>3} {:>3} {:>3}  {:>14}",
                a.id, a.name, a.current, a.worst, a.threshold, a.raw
            );
        }
    }
}
