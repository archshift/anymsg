//! A system for managing typed events
//!
//! # Example
//! ```
//! use std::thread;
//! use std::sync::atomic::{AtomicBool, Ordering};
//! use anymsg::{Pump, EventLoopMapping};
//!
//! let mut pump = Pump::new();
//!
//! // Define various messages
//! struct UpdateMsg { what: usize }
//! struct AddMsg { by: usize }
//! struct VerifyMsg { equals: usize }
//!
//! // Define our Client, with its own state
//! struct ReceiverType {
//!     counter: usize
//! }
//!
//! // Create our initial state
//! let receiver = ReceiverType { counter: 1 };
//!
//! #
//! # static RAN_VERIFY: AtomicBool = AtomicBool::new(false);
//!
//! let sender = pump.sender();
//! let mut mapping = EventLoopMapping::new();
//!
//! mapping.add_client(receiver)
//!     .add_handler(|this: &mut ReceiverType, msg: &UpdateMsg| {
//!         this.counter = msg.what;
//!     })
//!     .add_handler(|this: &mut ReceiverType, msg: &AddMsg| {
//!         this.counter += msg.by;
//!     })
//!     .add_handler(|this, msg: &VerifyMsg| {
//!         assert!(this.counter == msg.equals);
//!         println!("We passed the test!");
//! #        RAN_VERIFY.store(true, Ordering::SeqCst);
//!     });
//!
//! let event_loop = pump.event_loop(mapping);
//! let pthread = pump.start();
//! let client_thread = thread::spawn(move || {
//!     let mut event_loop = event_loop;
//!     while event_loop.poll_once().is_ok() {}
//! });
//!
//! // Send our messages and terminate
//! sender.send(UpdateMsg {what: 7});
//! sender.send(AddMsg {by: 1}).unwrap();
//! sender.send(VerifyMsg {equals: 8}).unwrap();
//! sender.hang_up();
//!
//! client_thread.join().unwrap();
//! # assert!(RAN_VERIFY.load(Ordering::SeqCst));
//! pthread.join();
//! ```

#[cfg(test)]
mod test;

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::ops::DerefMut;

use std::collections::BTreeMap as Map;

type MsgId = TypeId;
type ClientId = TypeId;
pub type AnyMsg = (MsgId, Arc<Any + Send + Sync>);

/// Thread which rebroadcasts messages from publishers to subscribers.
/// Completes when all [`Sender`s](struct.Sender.html) are dropped.
pub struct PumpThread {
    thread: thread::JoinHandle<()>,
}

impl PumpThread {
    /// Joins the underlying message pump thread.
    pub fn join(self) {
        self.thread.join().unwrap()
    }
}

/// The primary object for managing the message protocol's initial state.
pub struct Pump {
    incoming_txside: mpsc::Sender<AnyMsg>,
    incoming: mpsc::Receiver<AnyMsg>,
    outgoing: Map<MsgId, Vec<mpsc::Sender<AnyMsg>>>
}

impl Pump {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            incoming_txside: tx,
            incoming: rx,
            outgoing: Map::new()
        }
    }

    pub fn event_loop(&mut self, mapping: EventLoopMapping) -> EventLoop {
        let (mpsc_tx, mpsc_rx) = mpsc::channel();
        for msg_id in mapping.handler_map.keys() {
            self.outgoing.entry(*msg_id).or_insert(Vec::new())
                .push(mpsc_tx.clone());
        }
        EventLoop {
            rx: mpsc_rx,
            mappings: mapping
        }
    }

    pub fn sender(&self) -> Sender {
        Sender {
            sender: self.incoming_txside.clone()
        }
    }

    pub fn start(self) -> PumpThread {
        PumpThread {
            thread: thread::Builder::new().name("MessagePump".to_owned())
                                          .stack_size(1<<16).spawn(move || {
                drop(self.incoming_txside);
                let incoming = self.incoming;
                let mut outgoing = self.outgoing;
                for msg in incoming.iter() {
                    outgoing.entry(msg.0).or_insert(Vec::new())
                            .retain(|client| client.send(msg.clone()).is_ok())
                }
            }).unwrap(),
        }
    }
}

pub struct Sender {
    sender: mpsc::Sender<AnyMsg>
}

impl Sender {
    pub fn send<Msg: 'static + Send + Sync>(&self, msg: Msg) -> Result<(), mpsc::SendError<AnyMsg>> {
        let res = self.sender.send((TypeId::of::<Msg>(), Arc::new(msg)));
        res
    }

    /// Drops the sender, closing the connection.
    /// If all senders hang up, the PumpThread exits.
    pub fn hang_up(self) { }
}

struct HandlerPair {
    key: ClientId,
    func: Box<Fn(&mut Any, &Any) + Send>
}

/// Stores information linking message types with the clients that accept them.
/// Used by the [`EventLoop`](struct.EventLoop.html) to ensure proper delegation.
pub struct EventLoopMapping {
    client_map: Map<ClientId, Box<Any + Send>>,
    handler_map: Map<MsgId, Vec<HandlerPair>>
}

impl EventLoopMapping {
    pub fn new() -> Self {
        Self {
            client_map: Map::new(),
            handler_map: Map::new()
        }
    }

    pub fn add_client<'a, C: Any + Send>(&'a mut self, client: C) -> ELHandlerAdder<'a, C> {
        assert!(!self.client_map.contains_key(&TypeId::of::<C>()));
        self.client_map.insert(TypeId::of::<C>(), Box::new(client));

        ELHandlerAdder {
            handler_map: &mut self.handler_map,
            _spooky: PhantomData
        }
    }

    pub fn handle(&mut self, msg: AnyMsg) {
        let (ty, data) = msg;
        if let Some(msg_clients) = self.handler_map.get(&ty) {
            for &HandlerPair { key: client_ty, func: ref h } in msg_clients.iter() {
                let client = self.client_map.get_mut(&client_ty).unwrap().deref_mut();
                h(client, &*data);
            }
        }
    }
}

pub struct EventLoop {
    rx: mpsc::Receiver<AnyMsg>,
    mappings: EventLoopMapping
}

impl EventLoop {
    pub fn poll_once(&mut self) -> Result<(), ()> {
        let msg = self.rx.recv();
        if let Ok(msg) = msg {
            self.mappings.handle(msg);
            Ok(())
        } else {
            Err(())
        }
    }
}

pub struct ELHandlerAdder<'a, Client: Any> {
    handler_map: &'a mut Map<MsgId, Vec<HandlerPair>>,
    _spooky: PhantomData<Client>
}

impl<'a, Client: Any> ELHandlerAdder<'a, Client> {
    pub fn add_handler<Msg: Any>(&mut self, func: fn(&mut Client, &Msg)) -> &mut Self {
        let handler: Box<Fn(&mut Any, &Any) + Send> = Box::new(move |state, msg| {
            func(state.downcast_mut::<Client>().unwrap(),
                 msg.downcast_ref::<Msg>().unwrap())
        });

        self.handler_map.entry(TypeId::of::<Msg>())
            .or_insert(Vec::new())
            .push(HandlerPair { key: TypeId::of::<Client>(), func: handler });
        self
    }
}
