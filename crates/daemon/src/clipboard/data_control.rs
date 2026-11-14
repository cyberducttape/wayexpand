//! Native `{{clipboard}}` read over `ext-data-control-v1`.
//!
//! wl-clipboard 2.2.1 and older cannot bind this protocol. Where a compositor
//! offers only it (KWin 6), `wl-paste` falls back to mapping its own surface
//! to obtain keyboard focus, which moves focus away from the window the
//! expansion targets, and the output guard then refuses the expansion. Data
//! control needs no focus. Each read opens its own short-lived connection,
//! so nothing observes the clipboard between reads.

use std::io::Read;
use std::os::fd::OwnedFd;
use std::time::Instant;

use wayland_client::{
    event_created_child,
    protocol::{wl_callback, wl_registry, wl_seat},
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

/// Text types in preference order, matching what `wl-paste --type
/// text/plain;charset=utf-8` accepts.
const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];
const MAX_OFFERS: usize = 16;
const MAX_TYPES_PER_OFFER: usize = 64;

pub(super) enum NativeRead {
    /// No Wayland session or no data-control global: use another reader.
    Unsupported,
    /// Data control answered; `None` means there is no usable text.
    Done(Option<String>),
}

#[derive(Default)]
struct State {
    seat: Option<(u32, u32)>,
    manager: Option<(u32, u32)>,
    synced: bool,
    offers: Vec<(ExtDataControlOfferV1, Vec<String>)>,
    /// `Some(None)` is an empty clipboard; `None` means not reported yet.
    selection: Option<Option<ExtDataControlOfferV1>>,
    finished: bool,
    /// Requests for data this client offers, tagged by source.
    source_events: Vec<(SourceTag, SourceEvent)>,
}

/// Which of this client's selection sources an event belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceTag {
    Paste,
    Restore,
}

enum SourceEvent {
    Send { mime_type: String, fd: OwnedFd },
    Cancelled,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => state.seat = Some((name, version)),
                "ext_data_control_manager_v1" => state.manager = Some((name, version)),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.synced = true;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                if state.offers.len() < MAX_OFFERS {
                    state.offers.push((id, Vec::new()));
                } else {
                    id.destroy();
                }
            }
            ext_data_control_device_v1::Event::Selection { id } => state.selection = Some(id),
            ext_data_control_device_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            if let Some((_, types)) = state.offers.iter_mut().find(|(known, _)| known == offer) {
                if types.len() < MAX_TYPES_PER_OFFER {
                    types.push(mime_type);
                }
            }
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, SourceTag> for State {
    fn event(
        state: &mut Self,
        _: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        tag: &SourceTag,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let event = match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                SourceEvent::Send { mime_type, fd }
            }
            ext_data_control_source_v1::Event::Cancelled => SourceEvent::Cancelled,
            _ => return,
        };
        state.source_events.push((*tag, event));
    }
}

/// Dispatch until `done` holds. Returns false on timeout or a broken
/// connection; the caller treats both as "no clipboard text".
fn dispatch_until(
    connection: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    deadline: Instant,
    done: impl Fn(&State) -> bool,
) -> bool {
    loop {
        if queue.dispatch_pending(state).is_err() {
            return false;
        }
        if done(state) {
            return true;
        }
        if connection.flush().is_err() {
            return false;
        }
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let timeout = rustix::event::Timespec {
            tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: remaining.subsec_nanos().into(),
        };
        let fd = guard.connection_fd();
        let mut fds = [rustix::event::PollFd::new(
            &fd,
            rustix::event::PollFlags::IN,
        )];
        match rustix::event::poll(&mut fds, Some(&timeout)) {
            Err(rustix::io::Errno::INTR) => continue,
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        if guard.read().is_err() {
            return false;
        }
    }
}

/// Read the selection as text, or report that data control is unavailable.
pub(super) fn read(deadline: Instant, max_bytes: usize) -> NativeRead {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return NativeRead::Unsupported;
    }
    let Ok(connection) = Connection::connect_to_env() else {
        return NativeRead::Unsupported;
    };
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    let display = connection.display();
    let registry = display.get_registry(&handle, ());
    display.sync(&handle, ());
    let mut state = State::default();
    if !dispatch_until(&connection, &mut queue, &mut state, deadline, |state| {
        state.synced
    }) {
        return NativeRead::Unsupported;
    }
    let (Some((seat_name, seat_version)), Some((manager_name, _))) = (state.seat, state.manager)
    else {
        return NativeRead::Unsupported;
    };
    let seat: wl_seat::WlSeat = registry.bind(seat_name, seat_version.min(1), &handle, ());
    let manager: ExtDataControlManagerV1 = registry.bind(manager_name, 1, &handle, ());
    let device = manager.get_data_device(&seat, &handle, ());
    // The compositor announces the current selection right after the device
    // is created. The selection event may be queued before the selected offer's
    // MIME-type events, so don't treat the selection alone as a complete offer.
    let announced = dispatch_until(&connection, &mut queue, &mut state, deadline, |state| {
        state.finished || selection_complete(state)
    });
    let text = if announced {
        receive_text(&connection, &state, deadline, max_bytes)
    } else {
        None
    };
    for (offer, _) in state.offers.drain(..) {
        offer.destroy();
    }
    device.destroy();
    manager.destroy();
    let _ = connection.flush();
    NativeRead::Done(text)
}

/// Asks KDE's Klipper (and compatible clipboard managers) not to record the
/// offered text in clipboard history: snippets can be private.
const SECRET_HINT_TYPE: &str = "x-kde-passwordManagerHint";
/// Largest previous clipboard text that is saved and restored after a paste.
const MAX_RESTORE_BYTES: usize = 4 * 1024 * 1024;
/// How long to keep answering requests after the first one, for terminals
/// that ask more than once (for example for several MIME types).
const PASTE_GRACE: std::time::Duration = std::time::Duration::from_millis(300);

/// Progress of a paste session, sent to the output worker.
pub(super) enum PasteEvent {
    /// The snippet is now the clipboard selection; press the paste key.
    Ready,
    /// Paste mode cannot be used; nothing on the clipboard was changed.
    Unavailable(&'static str),
    /// The focused application read the snippet from the clipboard.
    Served,
    /// The previous clipboard contents are back (or the selection was
    /// cleared); another paste session may start.
    Restored,
}

/// Run one paste session on this thread: save the current clipboard text,
/// offer `text`, serve the application's request, then restore the saved
/// text and keep serving it until another client takes the clipboard.
/// `abort` stops the session early; the clipboard is restored either way.
pub(super) fn paste_session(
    text: zeroize::Zeroizing<String>,
    events: std::sync::mpsc::Sender<PasteEvent>,
    abort: std::sync::mpsc::Receiver<()>,
    prepare_deadline: Instant,
) {
    let unavailable = |reason| {
        let _ = events.send(PasteEvent::Unavailable(reason));
    };
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return unavailable("no Wayland display");
    }
    let Ok(connection) = Connection::connect_to_env() else {
        return unavailable("cannot connect to the Wayland display");
    };
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    let display = connection.display();
    let registry = display.get_registry(&handle, ());
    display.sync(&handle, ());
    let mut state = State::default();
    if !dispatch_until(
        &connection,
        &mut queue,
        &mut state,
        prepare_deadline,
        |state| state.synced,
    ) {
        return unavailable("the compositor did not answer");
    }
    let (Some((seat_name, seat_version)), Some((manager_name, _))) = (state.seat, state.manager)
    else {
        return unavailable("the compositor does not offer ext-data-control-v1");
    };
    let seat: wl_seat::WlSeat = registry.bind(seat_name, seat_version.min(1), &handle, ());
    let manager: ExtDataControlManagerV1 = registry.bind(manager_name, 1, &handle, ());
    let device = manager.get_data_device(&seat, &handle, ());
    let announced = dispatch_until(
        &connection,
        &mut queue,
        &mut state,
        prepare_deadline,
        |state| state.finished || selection_complete(state),
    );
    if !announced || state.finished {
        return unavailable("the current clipboard selection was not announced");
    }
    // Save what to restore. Never replace a selection that cannot be put
    // back (an image, files, or unreadable text): typing loses nothing.
    let previous = match state.selection.as_ref() {
        Some(None) => None,
        Some(Some(_)) => {
            match receive_text(&connection, &state, prepare_deadline, MAX_RESTORE_BYTES) {
                Some(text) => Some(zeroize::Zeroizing::new(text)),
                None => return unavailable("the clipboard holds data that cannot be restored"),
            }
        }
        None => return unavailable("the current clipboard selection was not announced"),
    };
    for (offer, _) in state.offers.drain(..) {
        offer.destroy();
    }

    let paste_source = manager.create_data_source(&handle, SourceTag::Paste);
    for mime_type in TEXT_TYPES {
        paste_source.offer(mime_type.to_owned());
    }
    paste_source.offer(SECRET_HINT_TYPE.to_owned());
    device.set_selection(Some(&paste_source));
    state.synced = false;
    display.sync(&handle, ());
    if !dispatch_until(
        &connection,
        &mut queue,
        &mut state,
        prepare_deadline,
        |state| state.synced,
    ) {
        device.set_selection(None);
        let _ = connection.flush();
        return unavailable("the compositor did not accept the clipboard selection");
    }
    if events.send(PasteEvent::Ready).is_err() {
        restore(
            &connection,
            &mut queue,
            &mut state,
            &manager,
            &device,
            previous,
            &events,
        );
        return;
    }

    // Serve the paste until the worker gives up, another client takes the
    // clipboard, or the grace period after the first request ends.
    let mut served_at: Option<Instant> = None;
    loop {
        if abort.try_recv().is_ok() {
            break;
        }
        if served_at.is_some_and(|at| at.elapsed() >= PASTE_GRACE) {
            break;
        }
        let tick = Instant::now() + std::time::Duration::from_millis(20);
        let alive = dispatch_until(&connection, &mut queue, &mut state, tick, |state| {
            !state.source_events.is_empty()
        });
        let mut cancelled = false;
        for (tag, event) in std::mem::take(&mut state.source_events) {
            match (tag, event) {
                (SourceTag::Paste, SourceEvent::Send { mime_type, fd }) => {
                    let payload: &[u8] = if mime_type == SECRET_HINT_TYPE {
                        b"secret"
                    } else {
                        text.as_bytes()
                    };
                    write_all_with_deadline(fd, payload, Instant::now() + PASTE_WRITE_TIMEOUT);
                    if mime_type != SECRET_HINT_TYPE && served_at.is_none() {
                        served_at = Some(Instant::now());
                        let _ = events.send(PasteEvent::Served);
                    }
                }
                (SourceTag::Paste, SourceEvent::Cancelled) => cancelled = true,
                _ => {}
            }
        }
        if cancelled {
            // Another client owns the clipboard now; leave it alone.
            paste_source.destroy();
            device.destroy();
            manager.destroy();
            let _ = connection.flush();
            let _ = events.send(PasteEvent::Restored);
            return;
        }
        if !alive && connection.flush().is_err() {
            return;
        }
    }
    paste_source.destroy();
    restore(
        &connection,
        &mut queue,
        &mut state,
        &manager,
        &device,
        previous,
        &events,
    );
}

/// How long one paste or restore request may take to write.
const PASTE_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

fn selection_complete(state: &State) -> bool {
    match state.selection.as_ref() {
        Some(None) => true,
        Some(Some(selected)) => state
            .offers
            .iter()
            .any(|(offer, mime_types)| offer == selected && !mime_types.is_empty()),
        None => false,
    }
}

/// Put the saved clipboard text back (or clear the selection if it was
/// empty), then keep serving it until another client takes the clipboard.
fn restore(
    connection: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    manager: &ExtDataControlManagerV1,
    device: &ExtDataControlDeviceV1,
    previous: Option<zeroize::Zeroizing<String>>,
    events: &std::sync::mpsc::Sender<PasteEvent>,
) {
    let handle = queue.handle();
    let Some(previous) = previous else {
        device.set_selection(None);
        let _ = connection.flush();
        let _ = events.send(PasteEvent::Restored);
        return;
    };
    let source = manager.create_data_source(&handle, SourceTag::Restore);
    for mime_type in TEXT_TYPES {
        source.offer(mime_type.to_owned());
    }
    device.set_selection(Some(&source));
    let flushed = connection.flush().is_ok();
    let _ = events.send(PasteEvent::Restored);
    if !flushed {
        return;
    }
    loop {
        // Wake up periodically only to notice a dead connection.
        let tick = Instant::now() + std::time::Duration::from_secs(60);
        let alive = dispatch_until(connection, queue, state, tick, |state| {
            !state.source_events.is_empty()
        });
        for (tag, event) in std::mem::take(&mut state.source_events) {
            match (tag, event) {
                (SourceTag::Restore, SourceEvent::Send { fd, .. }) => write_all_with_deadline(
                    fd,
                    previous.as_bytes(),
                    Instant::now() + PASTE_WRITE_TIMEOUT,
                ),
                (SourceTag::Restore, SourceEvent::Cancelled) => {
                    source.destroy();
                    let _ = connection.flush();
                    return;
                }
                _ => {}
            }
        }
        if !alive && connection.flush().is_err() {
            return;
        }
    }
}

/// Write `bytes` to a requesting client's pipe, giving up at `deadline` so
/// a client that never reads cannot stall the session.
fn write_all_with_deadline(fd: OwnedFd, mut bytes: &[u8], deadline: Instant) {
    use std::io::Write;
    if rustix::fs::fcntl_setfl(&fd, rustix::fs::OFlags::NONBLOCK).is_err() {
        return;
    }
    let mut file = std::fs::File::from(fd);
    while !bytes.is_empty() {
        match file.write(bytes) {
            Ok(0) => return,
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return;
                }
                let timeout = rustix::event::Timespec {
                    tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
                    tv_nsec: remaining.subsec_nanos().into(),
                };
                let mut fds = [rustix::event::PollFd::new(
                    &file,
                    rustix::event::PollFlags::OUT,
                )];
                match rustix::event::poll(&mut fds, Some(&timeout)) {
                    Err(rustix::io::Errno::INTR) => {}
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
            }
            Err(_) => return,
        }
    }
}

fn receive_text(
    connection: &Connection,
    state: &State,
    deadline: Instant,
    max_bytes: usize,
) -> Option<String> {
    let offer = state.selection.as_ref()?.as_ref()?;
    let types = &state.offers.iter().find(|(known, _)| known == offer)?.1;
    let mime_type = TEXT_TYPES
        .iter()
        .find(|wanted| types.iter().any(|offered| offered == *wanted))?;
    let (reader, writer) = std::io::pipe().ok()?;
    offer.receive((*mime_type).to_owned(), std::os::fd::AsFd::as_fd(&writer));
    connection.flush().ok()?;
    // Only the clipboard owner may hold the write end, or EOF never comes.
    drop(writer);
    read_until_eof(OwnedFd::from(reader), deadline, max_bytes)
}

fn read_until_eof(fd: OwnedFd, deadline: Instant, max_bytes: usize) -> Option<String> {
    rustix::fs::fcntl_setfl(&fd, rustix::fs::OFlags::NONBLOCK).ok()?;
    let mut file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > max_bytes {
                    return None;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                let timeout = rustix::event::Timespec {
                    tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
                    tv_nsec: remaining.subsec_nanos().into(),
                };
                let mut fds = [rustix::event::PollFd::new(
                    &file,
                    rustix::event::PollFlags::IN,
                )];
                match rustix::event::poll(&mut fds, Some(&timeout)) {
                    Err(rustix::io::Errno::INTR) => {}
                    Ok(0) | Err(_) => return None,
                    Ok(_) => {}
                }
            }
            Err(_) => return None,
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn pipe_reads_stop_at_the_size_limit_and_deadline() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(b"copied text").unwrap();
        drop(writer);
        let deadline = Instant::now() + Duration::from_millis(150);
        assert_eq!(
            read_until_eof(reader.into(), deadline, 64).as_deref(),
            Some("copied text")
        );

        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(&[b'x'; 65]).unwrap();
        drop(writer);
        assert_eq!(read_until_eof(reader.into(), deadline, 64), None);

        // An owner that never closes its end cannot hold the read past the
        // deadline.
        let (reader, _writer) = std::io::pipe().unwrap();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(50);
        assert_eq!(read_until_eof(reader.into(), deadline, 64), None);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn non_utf8_clipboard_text_is_rejected() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(&[0xff, 0xfe]).unwrap();
        drop(writer);
        let deadline = Instant::now() + Duration::from_millis(150);
        assert_eq!(read_until_eof(reader.into(), deadline, 64), None);
    }
}
