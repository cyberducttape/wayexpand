//! `wx-e2e-eis-relay SOCKET_PATH`
//!
//! Exposes KWin's emulated-input (EIS) server on a Unix socket path, so
//! WayExpand's libei backend and the e2e typist can reach it through
//! `LIBEI_SOCKET` inside a private headless session.
//!
//! KWin hands out EIS connections over D-Bus
//! (`org.kde.KWin.EIS.RemoteDesktop.connectToEIS`), which is what the KDE
//! RemoteDesktop portal calls after the user consents. In the private test
//! session nobody else is on that bus, so no consent dialog is involved.
//! Each client accepted on SOCKET_PATH gets its own EIS connection, and
//! bytes and passed file descriptors (keymaps) are relayed both ways.

use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::net::UnixListener,
    },
    thread,
};

use anyhow::{bail, Context, Result};
use rustix::net::{
    recvmsg, sendmsg, shutdown, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags,
    SendAncillaryBuffer, SendAncillaryMessage, SendFlags, Shutdown,
};

/// `connectToEIS` capability bits: keyboard, pointer, touch.
const EIS_CAPABILITIES: i32 = 1 | 2 | 4;
const MAX_PASSED_FDS: usize = 8;

fn connect_to_eis(bus: &zbus::blocking::Connection) -> Result<OwnedFd> {
    let reply = bus
        .call_method(
            Some("org.kde.KWin"),
            "/org/kde/KWin/EIS/RemoteDesktop",
            Some("org.kde.KWin.EIS.RemoteDesktop"),
            "connectToEIS",
            &(EIS_CAPABILITIES,),
        )
        .context("calling org.kde.KWin.EIS.RemoteDesktop.connectToEIS")?;
    let (fd, _cookie): (zbus::zvariant::OwnedFd, i32) = reply.body().deserialize()?;
    let fd: OwnedFd = fd.into();
    // libeis creates its end non-blocking; the relay's threads block instead.
    make_blocking(&fd)?;
    Ok(fd)
}

fn make_blocking(fd: &OwnedFd) -> Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, flags - rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}

/// Copy one direction until EOF, forwarding passed descriptors with the
/// bytes they arrived with.
fn pump(from: OwnedFd, to: OwnedFd) {
    let mut data = vec![0_u8; 64 * 1024];
    loop {
        let mut receive_space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_PASSED_FDS))];
        let mut received = RecvAncillaryBuffer::new(&mut receive_space);
        let message = match recvmsg(
            &from,
            &mut [IoSliceMut::new(&mut data)],
            &mut received,
            RecvFlags::CMSG_CLOEXEC,
        ) {
            Ok(message) => message,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => break,
        };
        let mut descriptors: Vec<OwnedFd> = Vec::new();
        for item in received.drain() {
            if let RecvAncillaryMessage::ScmRights(rights) = item {
                descriptors.extend(rights);
            }
        }
        if message.bytes == 0 && descriptors.is_empty() {
            break;
        }
        let borrowed: Vec<BorrowedFd<'_>> = descriptors.iter().map(AsFd::as_fd).collect();
        let mut send_space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_PASSED_FDS))];
        let mut ancillary = SendAncillaryBuffer::new(&mut send_space);
        if !borrowed.is_empty() && !ancillary.push(SendAncillaryMessage::ScmRights(&borrowed)) {
            eprintln!("wx-e2e-eis-relay: too many descriptors in one message");
            break;
        }
        let mut sent = 0;
        let mut first = true;
        while sent < message.bytes || (first && !borrowed.is_empty()) {
            let mut no_ancillary = SendAncillaryBuffer::default();
            let result = sendmsg(
                &to,
                &[IoSlice::new(&data[sent..message.bytes])],
                if first {
                    &mut ancillary
                } else {
                    &mut no_ancillary
                },
                SendFlags::NOSIGNAL,
            );
            match result {
                Ok(count) => {
                    sent += count;
                    first = false;
                }
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return,
            }
        }
    }
    let _ = shutdown(&to, Shutdown::Write);
}

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let (Some(path), None) = (arguments.next(), arguments.next()) else {
        bail!("usage: wx-e2e-eis-relay SOCKET_PATH");
    };
    let bus = zbus::blocking::Connection::session().context("connecting to the session bus")?;
    // Fail at start-up, not on the first client, if KWin offers no EIS.
    drop(connect_to_eis(&bus)?);
    let listener = UnixListener::bind(&path).with_context(|| format!("binding {path}"))?;
    eprintln!("wx-e2e-eis-relay: listening on {path}");
    for client in listener.incoming() {
        let client: OwnedFd = match client {
            Ok(client) => {
                let client: OwnedFd = client.into();
                make_blocking(&client)?;
                client
            }
            Err(error) => {
                eprintln!("wx-e2e-eis-relay: accept failed: {error}");
                continue;
            }
        };
        let server = match connect_to_eis(&bus) {
            Ok(server) => server,
            Err(error) => {
                eprintln!("wx-e2e-eis-relay: {error:#}");
                continue;
            }
        };
        let (client_in, server_in) = (client.try_clone()?, server.try_clone()?);
        thread::spawn(move || pump(client_in, server));
        thread::spawn(move || pump(server_in, client));
    }
    Ok(())
}
