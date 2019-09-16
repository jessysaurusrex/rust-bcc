use bcc::core::BPF;
use clap::{App, Arg};
use failure::Error;

use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{mem, thread, time};

// A simple tool for reporting runqueue latency
//
// Based on: https://github.com/iovisor/bcc/blob/master/tools/runqlat.py

fn do_main(runnable: Arc<AtomicBool>) -> Result<(), Error> {
    let matches = App::new("runqlat")
        .about("Reports distribution of scheduler latency")
        .arg(
            Arg::with_name("interval")
                .long("interval")
                .value_name("Seconds")
                .help("Integration window duration and period for stats output")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("windows")
                .long("windows")
                .value_name("Count")
                .help("The number of intervals before exit")
                .takes_value(true),
        )
        .get_matches();

    let interval: usize = matches
        .value_of("interval")
        .unwrap_or("1")
        .parse()
        .expect("Invalid argument for interval");
    let windows: Option<usize> = matches
        .value_of("windows")
        .map(|v| v.parse().expect("Invalid argument for windows"));

    let code = include_str!("runqlat.c");
    // compile the above BPF code!
    let mut bpf = BPF::new(code)?;

    // load + attach kprobes!
    let trace_run = bpf.load_kprobe("trace_run")?;
    let trace_ttwu_do_wakeup = bpf.load_kprobe("trace_ttwu_do_wakeup")?;
    let trace_wake_up_new_task = bpf.load_kprobe("trace_wake_up_new_task")?;

    bpf.attach_kprobe("finish_task_switch", trace_run)?;
    bpf.attach_kprobe("wake_up_new_task", trace_wake_up_new_task)?;
    bpf.attach_kprobe("ttwu_do_wakeup", trace_ttwu_do_wakeup)?;

    let table = bpf.table("dist");
    let mut window = 0;

    while runnable.load(Ordering::SeqCst) {
        thread::sleep(time::Duration::new(interval as u64, 0));
        println!("======");
        let mut overflow = 0;
        for (power, entry) in table.iter().enumerate() {
            let value = entry.value;

            let mut v = [0_u8; 8];
            for i in 0..8 {
                v[i] = *value.get(i).unwrap_or(&0);
            }
            let count: u64 = unsafe { mem::transmute(v) };
            let value = 2_u64.pow(power as u32);
            if value < 1_000_000 {
                println!("{} uS: {}", 2_u64.pow(power as u32), count);
            } else {
                overflow += count;
            }
        }
        println!("> 1 S: {}", overflow);
        if let Some(windows) = windows {
            window += 1;
            if window >= windows {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn main() {
    let runnable = Arc::new(AtomicBool::new(true));
    let r = runnable.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Failed to set handler for SIGINT / SIGTERM");

    match do_main(runnable) {
        Err(x) => {
            eprintln!("Error: {}", x);
            eprintln!("{}", x.backtrace());
            std::process::exit(1);
        }
        _ => {}
    }
}