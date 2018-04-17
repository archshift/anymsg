use *;

use std::thread;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};

#[test]
pub fn test_multi() {
    let mut pump = Pump::new();
    struct Client1(usize);
    struct Client2(usize);
    struct MsgIncr;
    struct MsgVerify;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    static RAN_VERIFY1: AtomicBool = AtomicBool::new(false);
    static RAN_VERIFY2: AtomicBool = AtomicBool::new(false);

    let mut mappings = EventLoopMapping::new();
    mappings.add_client(Client1(4))
        .add_handler(|this, _: &MsgIncr| {
            COUNTER.fetch_add(1, Ordering::SeqCst);
            this.0 += 1;
        })
        // Can have multiple handlers for the same (Client,Msg) pair
        .add_handler(|this, _: &MsgIncr| {
            COUNTER.fetch_add(1, Ordering::SeqCst);
            this.0 += 1;
        })
        .add_handler(|this, _: &MsgVerify| {
            assert!(this.0 == 6);
            RAN_VERIFY1.store(true, Ordering::SeqCst);
        });

    mappings.add_client(Client2(3))
        .add_handler(|this, _: &MsgIncr| {
            COUNTER.fetch_add(5, Ordering::SeqCst);
            this.0 += 5;
        })
        .add_handler(|this, _: &MsgVerify| {
            assert!(this.0 == 8);
            RAN_VERIFY2.store(true, Ordering::SeqCst);
        });

    let mut event_loop = pump.event_loop(mappings);
    let sender = pump.sender();
    let pthread = pump.start();

    sender.send(MsgIncr).unwrap();
    sender.send(MsgVerify).unwrap();
    sender.hang_up();

    thread::spawn(move || while event_loop.poll_once().is_ok() {})
        .join().unwrap();

    assert!(COUNTER.load(Ordering::SeqCst) == 7);
    assert!(RAN_VERIFY1.load(Ordering::SeqCst));
    assert!(RAN_VERIFY2.load(Ordering::SeqCst));

    pthread.join();
}
