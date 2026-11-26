// SPDX-License-Identifier: GPL-3.0-or-later
//! Built's end of the Blender link: a loopback TCP listener that accepts one
//! add-on at a time. The add-on's first message is `hello` with the pairing
//! token; a wrong token, another protocol or another document's `.blend` is
//! refused and the socket closed. Reading and writing run on their own
//! threads, so the UI thread only queues and drains (`send`, `poll`).
use super::wire::{self, Assembler, Message, PROTOCOL};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// What the UI thread learns from `poll`.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A message from the linked add-on.
    Message(Message),
    /// The add-on passed `hello`; its header (file, blender, document...).
    Linked(Map<String, Value>),
    /// A connection was refused: reason and what the add-on said.
    Refused(String, Map<String, Value>),
    /// The linked add-on went away (closed, crashed, unlinked).
    Closed(String),
}

struct Link {
    id: u64,
    stream: TcpStream,
    outgoing: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
    queued: Arc<AtomicUsize>,
}

#[derive(Default)]
struct Identity {
    document: String,
    name: String,
}

struct Shared {
    token: String,
    stop: AtomicBool,
    events: Mutex<VecDeque<Event>>,
    link: Mutex<Option<Link>>,
    identity: Mutex<Identity>,
    next_link: AtomicU64,
    next_msg: AtomicU64,
    hello_timeout: Duration,
}

pub struct Server {
    shared: Arc<Shared>,
    port: u16,
    #[cfg_attr(not(test), allow(dead_code))]
    address: SocketAddr,
    accept: Option<thread::JoinHandle<()>>,
}

fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn write_message(stream: &mut TcpStream, m: &Message, msg: u64) -> std::io::Result<()> {
    for f in wire::encode(m, msg, wire::CHUNK) {
        stream.write_all(&f)?;
    }
    stream.flush()
}

impl Server {
    /// Listen on 127.0.0.1:`port` (0: any free port; a taken port falls back
    /// to any free one).
    pub fn start(token: String, port: u16) -> std::io::Result<Self> {
        Self::start_with(token, port, Duration::from_secs(10))
    }

    pub fn start_with(token: String, port: u16, hello_timeout: Duration) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .or_else(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let port = address.port();
        let shared = Arc::new(Shared {
            token,
            stop: AtomicBool::new(false),
            events: Mutex::new(VecDeque::new()),
            link: Mutex::new(None),
            identity: Mutex::new(Identity::default()),
            next_link: AtomicU64::new(1),
            next_msg: AtomicU64::new(1),
            hello_timeout,
        });
        let s = shared.clone();
        let accept = thread::Builder::new()
            .name("blender-link-accept".into())
            .spawn(move || accept_loop(listener, s))?;
        Ok(Self { shared, port, address, accept: Some(accept) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    /// The document an add-on must name to link (empty: any).
    pub fn set_document(&self, id: &str, name: &str) {
        let mut i = self.shared.identity.lock().unwrap();
        i.document = id.to_owned();
        i.name = name.to_owned();
    }
    pub fn linked(&self) -> bool {
        self.shared.link.lock().unwrap().is_some()
    }
    /// Bytes waiting to be written to the add-on.
    pub fn queued(&self) -> usize {
        self.shared.link.lock().unwrap().as_ref().map_or(0, |l| l.queued.load(Ordering::Relaxed))
    }
    pub fn poll(&self) -> Vec<Event> {
        self.shared.events.lock().unwrap().drain(..).collect()
    }
    /// Queue a message for the linked add-on; false when none is linked.
    pub fn send(&self, m: &Message) -> bool {
        let guard = self.shared.link.lock().unwrap();
        let Some(link) = guard.as_ref() else {
            return false;
        };
        let id = self.shared.next_msg.fetch_add(1, Ordering::Relaxed);
        let frames = wire::encode(m, id, wire::CHUNK);
        let (queue, ready) = &*link.outgoing;
        let mut q = queue.lock().unwrap();
        for f in frames {
            link.queued.fetch_add(f.len(), Ordering::Relaxed);
            q.push_back(f);
        }
        ready.notify_one();
        true
    }
    /// Close the current link (Unlink); the listener keeps listening.
    pub fn disconnect(&self, reason: &str) {
        let link = self.shared.link.lock().unwrap().take();
        if let Some(link) = link {
            let _ = link.stream.shutdown(Shutdown::Both);
            link.outgoing.1.notify_all();
            self.shared.events.lock().unwrap().push_back(Event::Closed(reason.to_owned()));
        }
    }
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(link) = self.shared.link.lock().unwrap().take() {
            let _ = link.stream.shutdown(Shutdown::Both);
            link.outgoing.1.notify_all();
        }
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, addr)) => {
                // Loopback only: the bind already guarantees it; refuse anything else.
                if !addr.ip().is_loopback() {
                    continue;
                }
                let s = shared.clone();
                let _ = thread::Builder::new()
                    .name("blender-link-conn".into())
                    .spawn(move || connection(stream, addr, s));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(40)),
            Err(_) => thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn refuse(stream: &mut TcpStream, shared: &Shared, reason: &str, message: &str, hello: Map<String, Value>) {
    let name = shared.identity.lock().unwrap().name.clone();
    let m = Message::new(object(json!({"type": "refused", "reason": reason, "message": message, "document_name": name})), Vec::new());
    let _ = write_message(stream, &m, 0);
    let _ = stream.shutdown(Shutdown::Both);
    shared.events.lock().unwrap().push_back(Event::Refused(reason.to_owned(), hello));
}

/// Read until whole messages arrive (or the deadline / stop).
fn read_first(stream: &mut TcpStream, asm: &mut Assembler, until: Instant, shared: &Shared) -> Option<Vec<Message>> {
    let mut buf = [0u8; 64 * 1024];
    while Instant::now() < until && !shared.stop.load(Ordering::SeqCst) {
        match stream.read(&mut buf) {
            Ok(0) => return None,
            Ok(n) => match asm.feed(&buf[..n]) {
                Ok(ms) if !ms.is_empty() => return Some(ms),
                Ok(_) => {}
                Err(_) => return None,
            },
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return None,
        }
    }
    None
}

fn connection(mut stream: TcpStream, _addr: SocketAddr, shared: Arc<Shared>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut asm = Assembler::new();
    let Some(mut first) = read_first(&mut stream, &mut asm, Instant::now() + shared.hello_timeout, &shared) else {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    };
    let hello = first.remove(0);
    let header = hello.header.clone();
    let field = |k: &str| header.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    if hello.kind() != "hello" || !wire::same_secret(&field("token"), &shared.token) {
        return refuse(&mut stream, &shared, "token", "The pairing code does not match. Copy it again from Built.", header);
    }
    if header.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL) {
        return refuse(&mut stream, &shared, "protocol", "This add-on speaks another version of the link. Install the add-on from this Built.", header);
    }
    let (document, name) = {
        let i = shared.identity.lock().unwrap();
        (i.document.clone(), i.name.clone())
    };
    let theirs = field("document");
    let relink = header.get("relink").and_then(Value::as_bool).unwrap_or(false);
    if !theirs.is_empty() && !document.is_empty() && theirs != document && !relink {
        let msg = format!("This .blend is linked to another Built model. Open that model in Built, or link this file to {name} instead.");
        return refuse(&mut stream, &shared, "document", &msg, header);
    }
    let Ok(writer) = stream.try_clone() else { return };
    let id = shared.next_link.fetch_add(1, Ordering::Relaxed);
    let outgoing = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
    let queued = Arc::new(AtomicUsize::new(0));
    {
        let mut slot = shared.link.lock().unwrap();
        if slot.is_some() {
            drop(slot);
            return refuse(&mut stream, &shared, "busy", "Built is already linked to another Blender session.", header);
        }
        let welcome = Message::new(object(json!({"type": "welcome", "protocol": PROTOCOL, "document": document, "name": name})), Vec::new());
        if write_message(&mut stream, &welcome, 0).is_err() {
            return;
        }
        *slot = Some(Link { id, stream: writer, outgoing: outgoing.clone(), queued: queued.clone() });
        let mut events = shared.events.lock().unwrap();
        events.push_back(Event::Linked(header));
        events.extend(first.into_iter().map(Event::Message));
    }
    let Ok(out_stream) = stream.try_clone() else { return };
    let s2 = shared.clone();
    let writer_thread = thread::Builder::new()
        .name("blender-link-write".into())
        .spawn(move || write_loop(out_stream, outgoing, queued, s2, id));
    let reason = read_loop(&mut stream, &mut asm, &shared, id);
    let _ = stream.shutdown(Shutdown::Both);
    let ours = {
        let mut slot = shared.link.lock().unwrap();
        if slot.as_ref().is_some_and(|l| l.id == id) { slot.take() } else { None }
    };
    if let Some(link) = ours {
        link.outgoing.1.notify_all();
        shared.events.lock().unwrap().push_back(Event::Closed(reason));
    }
    // Never join while holding the link: the writer checks it to stop.
    if let Ok(t) = writer_thread {
        let _ = t.join();
    }
}

fn current(shared: &Shared, id: u64) -> bool {
    shared.link.lock().unwrap().as_ref().is_some_and(|l| l.id == id)
}

fn read_loop(stream: &mut TcpStream, asm: &mut Assembler, shared: &Shared, id: u64) -> String {
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if shared.stop.load(Ordering::SeqCst) || !current(shared, id) {
            return "unlinked".into();
        }
        match stream.read(&mut buf) {
            Ok(0) => return "Blender closed the link".into(),
            Ok(n) => match asm.feed(&buf[..n]) {
                Ok(ms) => {
                    let mut events = shared.events.lock().unwrap();
                    for m in ms {
                        if m.kind() == "bye" {
                            return "Blender closed the link".into();
                        }
                        events.push_back(Event::Message(m));
                    }
                }
                Err(e) => return e.to_string(),
            },
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return "Blender closed the link".into(),
        }
    }
}

fn write_loop(
    mut stream: TcpStream,
    outgoing: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
    queued: Arc<AtomicUsize>,
    shared: Arc<Shared>,
    id: u64,
) {
    let (queue, ready) = &*outgoing;
    loop {
        // `send` takes the link then the queue: check the link first, never
        // while holding the queue.
        if shared.stop.load(Ordering::SeqCst) || !current(&shared, id) {
            return;
        }
        let frame = {
            let mut q = queue.lock().unwrap();
            if q.is_empty() {
                q = ready.wait_timeout(q, Duration::from_millis(250)).unwrap().0;
            }
            q.pop_front()
        };
        let Some(frame) = frame else { continue };
        let n = frame.len();
        if stream.write_all(&frame).is_err() {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        queued.fetch_sub(n, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Client {
        stream: TcpStream,
        asm: Assembler,
        ready: VecDeque<Message>,
    }
    impl Client {
        fn connect(port: u16, hello: Value) -> Self {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            write_message(&mut stream, &Message::new(object(hello), Vec::new()), 0).unwrap();
            Self { stream, asm: Assembler::new(), ready: VecDeque::new() }
        }
        fn next(&mut self) -> Option<Message> {
            let mut buf = [0u8; 65536];
            while self.ready.is_empty() {
                let n = self.stream.read(&mut buf).ok()?;
                if n == 0 {
                    return None;
                }
                self.ready.extend(self.asm.feed(&buf[..n]).ok()?);
            }
            self.ready.pop_front()
        }
    }

    fn wait(server: &Server, pred: impl Fn(&Event) -> bool) -> Vec<Event> {
        let until = Instant::now() + Duration::from_secs(5);
        let mut seen = Vec::new();
        while Instant::now() < until {
            seen.extend(server.poll());
            if seen.iter().any(&pred) {
                return seen;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("event not seen: {seen:?}");
    }

    fn hello(token: &str, document: &str) -> Value {
        json!({"type": "hello", "token": token, "protocol": PROTOCOL, "document": document, "file": "bench.blend"})
    }

    #[test]
    fn the_right_token_links_and_messages_flow_both_ways() {
        let server = Server::start("ABCD-EFGH".into(), 0).unwrap();
        server.set_document("doc-1", "bench.igz");
        let mut c = Client::connect(server.port(), hello("ABCD-EFGH", "doc-1"));
        let welcome = c.next().unwrap();
        assert_eq!(welcome.kind(), "welcome");
        assert_eq!(welcome.header["document"], "doc-1");
        wait(&server, |e| matches!(e, Event::Linked(h) if h["file"] == "bench.blend"));
        assert!(server.linked());
        let big: Vec<u8> = (0..3 * wire::CHUNK + 5).map(|i| i as u8).collect();
        assert!(server.send(&Message::new(object(json!({"type": "chunk", "uid": "u1"})), big.clone())));
        let got = c.next().unwrap();
        assert_eq!(got.kind(), "chunk");
        assert_eq!(got.payload, big);
        write_message(&mut c.stream, &Message::new(object(json!({"type": "changes", "items": []})), vec![9; 10]), 4).unwrap();
        wait(&server, |e| matches!(e, Event::Message(m) if m.kind() == "changes" && m.payload == vec![9; 10]));
        drop(c);
        wait(&server, |e| matches!(e, Event::Closed(_)));
        assert!(!server.linked());
    }

    #[test]
    fn a_wrong_token_is_refused_and_never_links() {
        let server = Server::start("ABCD-EFGH".into(), 0).unwrap();
        let mut c = Client::connect(server.port(), hello("ABCD-EFGX", ""));
        let refused = c.next().unwrap();
        assert_eq!(refused.kind(), "refused");
        assert_eq!(refused.header["reason"], "token");
        assert!(c.next().is_none(), "the socket closes");
        wait(&server, |e| matches!(e, Event::Refused(r, _) if r == "token"));
        assert!(!server.linked());
    }

    #[test]
    fn another_documents_blend_is_refused_unless_relinked() {
        let server = Server::start("T".into(), 0).unwrap();
        server.set_document("doc-1", "bench.igz");
        let mut c = Client::connect(server.port(), hello("T", "doc-2"));
        let r = c.next().unwrap();
        assert_eq!(r.header["reason"], "document");
        assert_eq!(r.header["document_name"], "bench.igz");
        let mut h = hello("T", "doc-2");
        h["relink"] = json!(true);
        let mut c = Client::connect(server.port(), h);
        assert_eq!(c.next().unwrap().kind(), "welcome");
    }

    #[test]
    fn a_second_blender_is_refused_while_one_is_linked_and_reconnect_works() {
        let server = Server::start("T".into(), 0).unwrap();
        let mut a = Client::connect(server.port(), hello("T", ""));
        assert_eq!(a.next().unwrap().kind(), "welcome");
        wait(&server, |e| matches!(e, Event::Linked(_)));
        let mut b = Client::connect(server.port(), hello("T", ""));
        assert_eq!(b.next().unwrap().header["reason"], "busy");
        server.disconnect("unlinked");
        assert!(a.next().is_none());
        let mut again = Client::connect(server.port(), hello("T", ""));
        assert_eq!(again.next().unwrap().kind(), "welcome");
    }

    #[test]
    fn a_silent_client_is_dropped_after_the_hello_timeout() {
        let server = Server::start_with("T".into(), 0, Duration::from_millis(300)).unwrap();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(s.read(&mut buf).unwrap_or(0), 0);
        assert!(!server.linked());
    }

    #[test]
    fn it_listens_on_loopback_only() {
        let server = Server::start("T".into(), 0).unwrap();
        assert!(server.address().ip().is_loopback());
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).is_ok());
    }

    #[test]
    fn sending_while_the_writer_waits_does_not_deadlock() {
        let server = Server::start("T".into(), 0).unwrap();
        let mut c = Client::connect(server.port(), hello("T", ""));
        assert_eq!(c.next().unwrap().kind(), "welcome");
        wait(&server, |e| matches!(e, Event::Linked(_)));
        for i in 0..200 {
            assert!(server.send(&Message::new(object(json!({"type": "transform", "i": i})), Vec::new())));
        }
        for i in 0..200 {
            assert_eq!(c.next().unwrap().header["i"], i);
        }
    }
}
