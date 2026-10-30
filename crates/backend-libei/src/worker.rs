//! Thread-affine owner for the libei session.
//!
//! `reis::event::EiEventConverter` is intentionally `!Send`. Construct it,
//! process it, and destroy it on one worker thread; callers receive only this
//! request/reply handle, whose messages are naturally `Send`.

use super::{ConnectCancellation, LibeiError, LibeiOptions, LibeiSession, BACKEND_NAME};
use std::{
    io,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};
use wayexpand_core::{
    InjectorCapabilities, InjectorError, KeyEventState, Modifiers, OutputCheckpoint, TextInjector,
};

type Reply = SyncSender<Result<(), InjectorError>>;

enum SessionCommand {
    Checkpoint(OutputCheckpoint),
    Preflight(String, Reply),
    Erase(String, Reply),
    Insert(String, Reply),
    Replace(String, String, Reply),
    MoveCursor(usize, Reply),
    InjectKey(u32, Reply),
    InjectKeyWithModifiers(u32, Modifiers, Reply),
    InjectKeyEvent(u32, Modifiers, KeyEventState, Reply),
    Shutdown,
}

/// `Send` handle for a libei session that remains owned by its EIS thread.
pub struct LibeiInjector {
    sender: Option<SyncSender<SessionCommand>>,
    worker: Option<JoinHandle<()>>,
    capabilities: InjectorCapabilities,
    status_detail: &'static str,
}

impl LibeiInjector {
    /// Start an EIS-owning thread, negotiate the session there, then return
    /// once its runtime capabilities are known. The thread remains the sole
    /// owner of the non-`Send` event converter until shutdown.
    pub fn connect(options: LibeiOptions) -> Result<Self, LibeiError> {
        Self::connect_with_cancellation(options, ConnectCancellation::default())
    }

    pub fn connect_with_cancellation(
        options: LibeiOptions,
        cancellation: ConnectCancellation,
    ) -> Result<Self, LibeiError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("wayexpand-libei-session".into())
            .spawn(
                move || match LibeiSession::connect_with_cancellation(options, cancellation) {
                    Ok(mut session) => {
                        let ready = (session.capabilities(), session.status_detail());
                        if ready_sender.send(Ok(ready)).is_err() {
                            session.shutdown();
                            return;
                        }
                        session_loop(&mut session, receiver);
                        session.shutdown();
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                    }
                },
            )
            .map_err(LibeiError::Connect)?;

        match ready_receiver.recv() {
            Ok(Ok((capabilities, status_detail))) => Ok(Self {
                sender: Some(sender),
                worker: Some(worker),
                capabilities,
                status_detail,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(LibeiError::Connect(io::Error::other(
                    "libei session thread ended before reporting readiness",
                )))
            }
        }
    }

    fn call(&self, request: impl FnOnce(Reply) -> SessionCommand) -> Result<(), InjectorError> {
        let (reply, response) = mpsc::sync_channel(1);
        let Some(sender) = self.sender.as_ref() else {
            return Err(worker_unavailable());
        };
        sender
            .send(request(reply))
            .map_err(|_| worker_unavailable())?;
        response
            .recv()
            .unwrap_or_else(|_| Err(worker_unavailable()))
    }
}

impl TextInjector for LibeiInjector {
    fn shutdown(mut self: Box<Self>) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(SessionCommand::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn capabilities(&self) -> InjectorCapabilities {
        self.capabilities
    }

    fn status_detail(&self) -> &'static str {
        self.status_detail
    }

    fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::Preflight(text.to_owned(), reply))
    }

    fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
        if let Some(sender) = self.sender.as_ref() {
            let _ = sender.send(SessionCommand::Checkpoint(checkpoint));
        }
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::Erase(trigger.to_owned(), reply))
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::Insert(text.to_owned(), reply))
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::Replace(trigger.to_owned(), text.to_owned(), reply))
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::MoveCursor(count, reply))
    }

    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::InjectKey(keycode, reply))
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::InjectKeyWithModifiers(keycode, modifiers, reply))
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        self.call(|reply| SessionCommand::InjectKeyEvent(keycode, modifiers, state, reply))
    }
}

impl Drop for LibeiInjector {
    fn drop(&mut self) {
        // Closing the command channel asks a healthy worker to close its
        // portal and EIS objects. Do not join in Drop: daemon shutdown wraps
        // explicit shutdown in its own deadline and may have to retire a
        // stalled worker.
        self.sender.take();
    }
}

fn session_loop(session: &mut LibeiSession, receiver: Receiver<SessionCommand>) {
    while let Ok(command) = receiver.recv() {
        let (reply, result) = match command {
            SessionCommand::Checkpoint(checkpoint) => {
                session.set_output_checkpoint(checkpoint);
                continue;
            }
            SessionCommand::Preflight(text, reply) => (reply, session.preflight_text(&text)),
            SessionCommand::Erase(trigger, reply) => (reply, session.erase(&trigger)),
            SessionCommand::Insert(text, reply) => (reply, session.insert(&text)),
            SessionCommand::Replace(trigger, text, reply) => {
                (reply, session.replace(&trigger, &text))
            }
            SessionCommand::MoveCursor(count, reply) => (reply, session.move_cursor_left(count)),
            SessionCommand::InjectKey(keycode, reply) => (reply, session.inject_key(keycode)),
            SessionCommand::InjectKeyWithModifiers(keycode, modifiers, reply) => {
                (reply, session.inject_key_with_modifiers(keycode, modifiers))
            }
            SessionCommand::InjectKeyEvent(keycode, modifiers, state, reply) => {
                (reply, session.inject_key_event(keycode, modifiers, state))
            }
            SessionCommand::Shutdown => break,
        };
        let _ = reply.send(result);
    }
}

fn worker_unavailable() -> InjectorError {
    InjectorError {
        backend: BACKEND_NAME,
        message: "the thread-affine libei session is unavailable".into(),
        retryable: true,
    }
}
