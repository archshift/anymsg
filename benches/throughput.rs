#[macro_use]
extern crate criterion;
extern crate anymsg;

use criterion::*;

use anymsg::*;
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn pub_events(b: &mut Bencher) {
    let mut pump = Pump::new();
    struct Client();
    struct MsgIncr;

    let sender = pump.sender();

    let mut mappings = EventLoopMapping::new();
    mappings.add_client(Client)
        .add_handler(|this, _: &MsgIncr| {
            black_box(());
        });

    let mut event_loop = pump.event_loop(mappings);
    let pthread = pump.start();

    let t = thread::spawn(move || while event_loop.poll_once().is_ok() {});

    b.iter(|| {
        sender.send(MsgIncr).unwrap();
    });
    sender.hang_up();

    t.join().unwrap();
    pthread.join();
}

pub fn receive_events(b: &mut Bencher, size: &usize) {
    let mut pump = Pump::new();
    struct Client(usize);
    struct MsgIncr;
    struct MsgFin;

    let sender = pump.sender();

    static DONE: AtomicBool = AtomicBool::new(false);

    let mut mappings = EventLoopMapping::new();
    mappings.add_client(Client(0))
        .add_handler(|this, _: &MsgIncr| {
            black_box(());
        })
        .add_handler(|this, _: &MsgFin| {
            DONE.store(true, Ordering::SeqCst);
        });

    let mut event_loop = pump.event_loop(mappings);
    let pthread = pump.start();

    let t = thread::spawn(move || while event_loop.poll_once().is_ok() {});

    b.iter(|| {
        for _ in 0..*size {
            sender.send(MsgIncr).unwrap();
        }
        sender.send(MsgFin).unwrap();
        while !DONE.load(Ordering::SeqCst) { }
        DONE.store(false, Ordering::SeqCst);
    });
    sender.hang_up();

    t.join().unwrap();
    pthread.join();
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function_over_inputs("msg xfer throughput", receive_events, vec![100, 500, 1000, 10000]);
    c.bench_function("msg publish throughput", pub_events);
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
