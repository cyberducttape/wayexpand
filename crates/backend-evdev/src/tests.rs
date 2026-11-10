use super::*;
use std::{
    io::{Read, Write},
    os::fd::BorrowedFd,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

struct SignalActionGuard {
    signal: libc::c_int,
    previous: libc::sigaction,
}

impl Drop for SignalActionGuard {
    fn drop(&mut self) {
        // SAFETY: `previous` was returned by sigaction for this signal
        // before the test installed its temporary handler.
        unsafe {
            libc::sigaction(self.signal, &self.previous, std::ptr::null_mut());
        }
    }
}

struct TestKeyboard {
    path: PathBuf,
    reader: UnixStream,
    events: Arc<Mutex<Vec<evdev::InputEvent>>>,
}

impl device::KeyboardDeviceAccess for TestKeyboard {
    fn path(&self) -> &Path {
        &self.path
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.reader.as_fd()
    }

    fn fetch_events(&mut self, output: &mut Vec<evdev::InputEvent>) -> std::io::Result<()> {
        let mut buffer = [0_u8; 64];
        let _ = self.reader.read(&mut buffer)?;
        output.clear();
        output.extend(std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        ));
        Ok(())
    }
}

struct TestLayoutProvider;

impl KeyboardLayoutProvider for TestLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        Some(KeyboardLayoutSnapshot {
            generation: 1,
            group: 0,
            rules: None,
            model: None,
            layout: None,
            variant: None,
            options: None,
        })
    }
}

struct MutableLayoutProvider(Arc<Mutex<Option<KeyboardLayoutSnapshot>>>);

impl KeyboardLayoutProvider for MutableLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        self.0.lock().unwrap().clone()
    }
}

fn test_state() -> State {
    let keymap = Keymap::new_from_names(Context::new(0).unwrap(), None, 0).unwrap();
    State::new(keymap)
}

fn source_with_provider(provider: Box<dyn KeyboardLayoutProvider>) -> EvdevSource {
    EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: provider,
        layout_generation: None,
        layout_available: false,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    }
}

#[test]
fn unavailable_layout_provider_fails_closed_but_tracks_physical_keys() {
    let mut source = source_with_provider(Box::new(UnavailableLayoutProvider));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
    assert_eq!(source.take_pending_events(), vec![InputEvent::Reset]);
    assert_eq!(
        source.input_diagnostics(),
        EvdevInputDiagnostics {
            raw_key_events_total: 1,
            layout_unavailable_key_events_total: 1,
        }
    );
    assert!(source.keys_held());
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0));
    assert!(!source.keys_held());
    assert_eq!(source.input_diagnostics().raw_key_events_total, 2);
    assert_eq!(
        source
            .input_diagnostics()
            .layout_unavailable_key_events_total,
        2
    );
    assert!(source.take_pending_events().is_empty());
}

#[test]
fn available_layout_counts_raw_keys_without_exposing_key_values() {
    let mut source = source_with_provider(Box::new(TestLayoutProvider));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));

    assert_eq!(
        source.input_diagnostics(),
        EvdevInputDiagnostics {
            raw_key_events_total: 1,
            layout_unavailable_key_events_total: 0,
        }
    );
    assert!(source
        .take_pending_events()
        .iter()
        .any(|event| matches!(event, InputEvent::TextChar('a'))));
}

#[test]
fn layout_generation_change_resets_matcher_and_replays_held_modifiers() {
    let snapshot = Arc::new(Mutex::new(Some(KeyboardLayoutSnapshot {
        generation: 1,
        group: 0,
        rules: None,
        model: None,
        layout: Some("us,de".into()),
        variant: None,
        options: None,
    })));
    let mut source = source_with_provider(Box::new(MutableLayoutProvider(Arc::clone(&snapshot))));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 42, 1));
    source.take_pending_events();
    *snapshot.lock().unwrap() = Some(KeyboardLayoutSnapshot {
        generation: 2,
        group: 1,
        rules: None,
        model: None,
        layout: Some("us,de".into()),
        variant: None,
        options: None,
    });
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 21, 1));
    let events = source.take_pending_events();
    assert_eq!(events.first(), Some(&InputEvent::Reset));
    assert!(
        events.contains(&InputEvent::TextChar('Z')),
        "events: {events:?}"
    );
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 42, 0));
}

#[test]
fn layout_change_preserves_locked_caps_state() {
    let snapshot = Arc::new(Mutex::new(Some(KeyboardLayoutSnapshot {
        generation: 1,
        group: 0,
        rules: None,
        model: Some("pc105".into()),
        layout: Some("us,de".into()),
        variant: Some(",".into()),
        options: None,
    })));
    let mut source = source_with_provider(Box::new(MutableLayoutProvider(Arc::clone(&snapshot))));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 58, 1));
    source.take_pending_events();
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 58, 0));
    source.take_pending_events();

    *snapshot.lock().unwrap() = Some(KeyboardLayoutSnapshot {
        generation: 2,
        group: 1,
        rules: None,
        model: Some("pc105".into()),
        layout: Some("us,de".into()),
        variant: Some(",".into()),
        options: None,
    });
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 21, 1));
    let events = source.take_pending_events();
    assert!(
        events.contains(&InputEvent::TextChar('Z')),
        "events: {events:?}"
    );
}

#[test]
fn eventfd_wakes_the_evdev_poll_and_is_drained() {
    let wake = rustix::event::eventfd(
        0,
        rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
    )
    .unwrap();
    let mut fds = [rustix::event::PollFd::new(
        &wake,
        rustix::event::PollFlags::IN,
    )];
    rustix::io::write(&wake, &1_u64.to_ne_bytes()).unwrap();

    assert!(poll_with_wake(&mut fds, Duration::from_secs(1), Some(&wake)).unwrap());
    assert!(!poll_with_wake(&mut fds, Duration::ZERO, Some(&wake)).unwrap());
}

const SIGNAL_TEST_CHILD_ENV: &str = "WAYEXPAND_EVDEV_SIGNAL_TEST_CHILD";

#[test]
fn a_signal_interrupting_the_poll_is_not_a_device_failure() {
    // Signal dispositions are process-global. Run the actual sigaction
    // manipulation in a one-test child process so it cannot race with
    // unrelated tests in this libtest process.
    let executable = std::env::current_exe().expect("locate evdev unit-test executable");
    let output = std::process::Command::new(executable)
        .args([
            "--ignored",
            "--exact",
            "tests::signal_interrupting_poll_subprocess_worker",
            "--nocapture",
        ])
        .env(SIGNAL_TEST_CHILD_ENV, "1")
        .output()
        .expect("run signal test in an isolated process");
    assert!(
        output.status.success(),
        "isolated signal test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("tests::signal_interrupting_poll_subprocess_worker ... ok"),
        "isolated test worker did not run: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
#[ignore = "helper executed only by the isolated signal test"]
fn signal_interrupting_poll_subprocess_worker() {
    assert_eq!(
        std::env::var(SIGNAL_TEST_CHILD_ENV).as_deref(),
        Ok("1"),
        "run this helper only through the subprocess-isolated parent test"
    );

    extern "C" fn ignore(_: libc::c_int) {}
    // SAFETY: installs a handler that does nothing, without SA_RESTART,
    // so the signal interrupts poll exactly as the daemon's handler does.
    let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = ignore as extern "C" fn(libc::c_int) as usize;
        libc::sigemptyset(&mut action.sa_mask);
        assert_eq!(libc::sigaction(libc::SIGUSR1, &action, &mut previous), 0);
    }
    let previous_handler = previous.sa_sigaction;
    let previous_flags = previous.sa_flags;
    let signal_guard = SignalActionGuard {
        signal: libc::SIGUSR1,
        previous,
    };
    let wake = rustix::event::eventfd(
        0,
        rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
    )
    .unwrap();
    let mut fds = [rustix::event::PollFd::new(
        &wake,
        rustix::event::PollFlags::IN,
    )];
    // SAFETY: pthread_self has no preconditions.
    let target = unsafe { libc::pthread_self() };
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signaller_done = Arc::clone(&done);
    // Signal until the poll returns, so a signal that lands before poll
    // starts cannot leave the test waiting for the full timeout.
    let signaller = std::thread::spawn(move || {
        while !signaller_done.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(50));
            // SAFETY: the test thread outlives this loop; it joins us.
            unsafe { libc::pthread_kill(target, libc::SIGUSR1) };
        }
    });
    let started = std::time::Instant::now();
    let woken = poll_with_wake(&mut fds, Duration::from_secs(10), Some(&wake));
    done.store(true, std::sync::atomic::Ordering::Release);
    signaller.join().unwrap();
    assert!(!woken.unwrap(), "an interrupted poll reports nothing ready");
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(signal_guard);

    // Confirm the process signal disposition was restored after the poll
    // test instead of leaking the temporary handler into later tests.
    let mut restored: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null new-action pointer queries the current disposition.
    assert_eq!(
        unsafe { libc::sigaction(libc::SIGUSR1, std::ptr::null(), &mut restored) },
        0
    );
    assert_eq!(restored.sa_sigaction, previous_handler);
    // Linux may add its private SA_RESTORER bit when returning the action.
    let public_flags = libc::SA_SIGINFO
        | libc::SA_RESTART
        | libc::SA_NODEFER
        | libc::SA_RESETHAND
        | libc::SA_ONSTACK
        | libc::SA_NOCLDSTOP
        | libc::SA_NOCLDWAIT;
    assert_eq!(
        restored.sa_flags & public_flags,
        previous_flags & public_flags
    );
}

#[test]
fn readable_fake_keyboard_events_are_processed_and_disconnect_resets_state() {
    let path = PathBuf::from("/dev/input/test-keyboard");
    let (reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut source = EvdevSource {
        devices: vec![Box::new(TestKeyboard {
            path: path.clone(),
            reader,
            events: Arc::clone(&events),
        })],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };

    events
        .lock()
        .unwrap()
        .push(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
    writer.write_all(b"event").unwrap();
    source.poll_once(Duration::from_secs(1), false).unwrap();
    assert!(source.keys_held());
    assert!(matches!(
        source.pending.pop_front(),
        Some(InputEvent::Key(_))
    ));
    assert_eq!(source.pending.pop_front(), Some(InputEvent::TextChar('a')));

    let warmed_capacities = POLL_BUFFERS.with(|buffers| {
        let buffers = buffers.borrow();
        (
            buffers.descriptors.capacity(),
            buffers.ready.capacity(),
            buffers.lost.capacity(),
        )
    });
    events
        .lock()
        .unwrap()
        .push(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0));
    writer.write_all(b"event").unwrap();
    source.poll_once(Duration::from_secs(1), false).unwrap();
    let reused_capacities = POLL_BUFFERS.with(|buffers| {
        let buffers = buffers.borrow();
        (
            buffers.descriptors.capacity(),
            buffers.ready.capacity(),
            buffers.lost.capacity(),
        )
    });
    assert_eq!(reused_capacities, warmed_capacities);

    drop(writer);
    source.poll_once(Duration::from_secs(1), false).unwrap();
    assert!(source.devices.is_empty());
    assert!(!source.keys_held());
    assert_eq!(source.pending.pop_front(), Some(InputEvent::Reset));
}

#[test]
fn output_monitor_observes_physical_input_without_refreshing_devices() {
    let path = PathBuf::from("/dev/input/test-keyboard");
    let (reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let events = Arc::new(Mutex::new(vec![evdev::InputEvent::new(
        evdev::EventType::KEY.0,
        30,
        1,
    )]));
    let mut source = EvdevSource {
        devices: vec![Box::new(TestKeyboard {
            path: path.clone(),
            reader,
            events,
        })],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    writer.write_all(b"event").unwrap();

    assert!(source.next_event_during_output(Duration::ZERO).unwrap());
    assert_eq!(source.pending.pop_front(), Some(InputEvent::TextChar('a')));
}

#[test]
fn input_topology_watch_observes_device_node_creation() {
    let directory =
        std::env::temp_dir().join(format!("wayexpand-evdev-topology-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    let watch = create_topology_watch(&directory).unwrap();

    std::fs::write(directory.join("event-test"), b"device").unwrap();
    let mut poll_fds = [rustix::event::PollFd::new(
        &watch,
        rustix::event::PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    assert_eq!(
        rustix::event::poll(&mut poll_fds, Some(&timeout)).unwrap(),
        1
    );
    assert!(poll_fds[0].revents().contains(rustix::event::PollFlags::IN));
    assert!(drain_topology_events(watch.as_fd()).unwrap());
    assert!(!drain_topology_events(watch.as_fd()).unwrap());

    drop(watch);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn input_topology_watch_invalidation_is_an_error() {
    let directory = std::env::temp_dir().join(format!(
        "wayexpand-evdev-watch-invalidation-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    let watch = create_topology_watch(&directory).unwrap();
    std::fs::remove_dir(&directory).unwrap();

    let mut poll_fds = [rustix::event::PollFd::new(
        &watch,
        rustix::event::PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    assert_eq!(
        rustix::event::poll(&mut poll_fds, Some(&timeout)).unwrap(),
        1
    );
    let error = drain_topology_events(watch.as_fd()).unwrap_err();
    assert!(matches!(error, EvdevError::HotplugMonitor(_)));

    drop(watch);
}

#[test]
fn hotplug_during_output_interrupts_before_device_discovery() {
    let directory =
        std::env::temp_dir().join(format!("wayexpand-evdev-interrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    let topology_watch = create_topology_watch(&directory).unwrap();
    let path = PathBuf::from("/dev/input/test-keyboard");
    let (reader, _writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut source = EvdevSource {
        devices: vec![Box::new(TestKeyboard {
            path,
            reader,
            events,
        })],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: Some(topology_watch),
        topology_refresh_pending: false,
    };

    std::fs::write(directory.join("event-new-keyboard"), b"device").unwrap();
    assert!(source
        .next_event_during_output(Duration::from_secs(1))
        .unwrap());
    assert!(source.topology_refresh_pending);
    assert_eq!(source.devices.len(), 1);

    drop(source);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn topology_notification_interrupts_before_a_buffered_raw_event_slice() {
    let directory = std::env::temp_dir().join(format!(
        "wayexpand-evdev-buffered-hotplug-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    let topology_watch = create_topology_watch(&directory).unwrap();
    let (reader, _writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let mut source = EvdevSource {
        devices: vec![Box::new(TestKeyboard {
            path: PathBuf::from("/dev/input/test-keyboard"),
            reader,
            events: Arc::new(Mutex::new(Vec::new())),
        })],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: (0..MAX_RAW_EVENTS_PER_SLICE * 4)
            .map(|_| evdev::InputEvent::new(evdev::EventType::SYNCHRONIZATION.0, 0, 0))
            .collect(),
        raw_event_cursor: 0,
        raw_event_device: Some(0),
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: Some(topology_watch),
        topology_refresh_pending: false,
    };

    std::fs::write(directory.join("event-new-keyboard"), b"device").unwrap();
    assert!(source.next_event_during_output(Duration::ZERO).unwrap());
    assert!(source.topology_refresh_pending);
    assert_eq!(source.raw_event_cursor, 0, "hotplug wins over raw backlog");

    drop(source);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn output_monitor_detects_modifier_activity_without_matcher_input() {
    let path = PathBuf::from("/dev/input/test-keyboard");
    let (reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let events = Arc::new(Mutex::new(vec![evdev::InputEvent::new(
        evdev::EventType::KEY.0,
        42, // KEY_LEFTSHIFT
        1,
    )]));
    let mut source = EvdevSource {
        devices: vec![Box::new(TestKeyboard {
            path: path.clone(),
            reader,
            events,
        })],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    writer.write_all(b"event").unwrap();

    assert!(source.next_event_during_output(Duration::ZERO).unwrap());
    assert!(source.pending.is_empty());
    assert!(source.keys_held());
}

#[test]
fn oversized_device_batches_are_translated_in_bounded_slices_without_loss() {
    let path = PathBuf::from("/dev/input/test-keyboard");
    let (reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let events = Arc::new(Mutex::new(
        (0..300)
            .map(|index| {
                evdev::InputEvent::new(
                    evdev::EventType::KEY.0,
                    30,
                    if index % 2 == 0 { 1 } else { 0 },
                )
            })
            .collect(),
    ));
    let mut source = EvdevSource {
        devices: vec![Box::new(TestKeyboard {
            path,
            reader,
            events,
        })],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    writer.write_all(b"events").unwrap();

    source.poll_once(Duration::ZERO, false).unwrap();
    assert_eq!(source.raw_event_cursor, MAX_RAW_EVENTS_PER_SLICE);
    let capacity_after_read = source.raw_events.capacity();
    let mut translated = source.pending.drain(..).count();

    while source.has_buffered_raw_events() {
        source.poll_once(Duration::ZERO, false).unwrap();
        translated += source.pending.drain(..).count();
    }

    assert_eq!(translated, 300, "every raw event must be translated");
    assert_eq!(source.raw_events.capacity(), capacity_after_read);
}

#[test]
fn event_slice_budget_does_not_hide_input_waiting_on_another_device() {
    let (reader_a, mut writer_a) = UnixStream::pair().unwrap();
    let (reader_b, mut writer_b) = UnixStream::pair().unwrap();
    reader_a.set_nonblocking(true).unwrap();
    reader_b.set_nonblocking(true).unwrap();
    let noise = Arc::new(Mutex::new(
        (0..MAX_RAW_EVENTS_PER_SLICE)
            .map(|_| evdev::InputEvent::new(evdev::EventType::SYNCHRONIZATION.0, 0, 0))
            .collect(),
    ));
    let key = Arc::new(Mutex::new(vec![evdev::InputEvent::new(
        evdev::EventType::KEY.0,
        30,
        1,
    )]));
    let mut source = EvdevSource {
        devices: vec![
            Box::new(TestKeyboard {
                path: PathBuf::from("/dev/input/test-a"),
                reader: reader_a,
                events: noise,
            }),
            Box::new(TestKeyboard {
                path: PathBuf::from("/dev/input/test-b"),
                reader: reader_b,
                events: key,
            }),
        ],
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::new(), HashSet::new()],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    writer_a.write_all(b"noise").unwrap();
    writer_b.write_all(b"key").unwrap();

    assert!(source.drain_ready(&[0, 1]).unwrap());
    assert!(!source
        .pending
        .iter()
        .any(|event| matches!(event, InputEvent::TextChar(_) | InputEvent::Text(_))));
    source.drain_ready(&[1]).unwrap();
    assert!(source.pending.contains(&InputEvent::TextChar('a')));
}

/// A press on a resolvable key always yields a candidate hotkey chord
/// first (mirroring `backend-input-method`; the engine ignores chords
/// that are not configured as a hotkey) followed by the ordinary
/// matcher action. Returns just the latter, which is what these tests
/// care about.
fn press(state: &mut State, code: u16) -> Option<InputEvent> {
    let event = evdev::InputEvent::new(evdev::EventType::KEY.0, code, 1);
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: std::mem::replace(state, test_state()),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    source.translate(event);
    *state = source.state;
    assert!(
        matches!(source.pending.pop_front(), Some(InputEvent::Key(_))),
        "expected a candidate hotkey chord to be queued first"
    );
    source.pending.pop_front()
}

#[test]
fn printable_key_becomes_scalar_character_event() {
    let mut state = test_state();
    // KEY_A = 30 in linux/input-event-codes.h.
    assert_eq!(press(&mut state, 30), Some(InputEvent::TextChar('a')));
}

#[test]
fn backspace_key_becomes_backspace_event() {
    let mut state = test_state();
    // KEY_BACKSPACE = 14.
    assert_eq!(press(&mut state, 14), Some(InputEvent::Backspace));
}

#[test]
fn enter_key_preserves_delimiter_event() {
    let mut state = test_state();
    // KEY_ENTER = 28.
    assert_eq!(press(&mut state, 28), Some(InputEvent::Delimiter('\n')));
}

#[test]
fn auto_repeat_forwards_text_without_a_hotkey_event() {
    let mut state = test_state();
    let event = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 2);
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: std::mem::replace(&mut state, test_state()),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    source.translate(event);
    assert_eq!(source.pending.pop_front(), Some(InputEvent::TextChar('a')));
    assert_eq!(source.pending.pop_front(), None);
}

#[test]
fn release_events_do_not_emit_text() {
    let mut state = test_state();
    let event = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0);
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: std::mem::replace(&mut state, test_state()),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    source.translate(event);
    assert_eq!(source.pending.pop_front(), None);
}

#[test]
fn held_keys_are_tracked_until_released() {
    let mut state = test_state();
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: std::mem::replace(&mut state, test_state()),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    assert!(!source.keys_held());

    // KEY_LEFTSHIFT = 42, KEY_A = 30, KEY_B = 48. Overlapping presses,
    // including a modifier held across ordinary key releases, must all
    // remain visible: injecting while any physical key is down can alter
    // or consume replacement keystrokes.
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 42, 1));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
    assert!(source.keys_held());
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 48, 1));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0));
    assert!(source.keys_held(), "the second key is still down");
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 48, 0));
    assert!(source.keys_held(), "left shift is still physically held");
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 42, 0));
    assert!(!source.keys_held());
}

#[test]
fn identical_keycodes_remain_held_across_devices() {
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    let keyboard_a = 0;
    let keyboard_b = 1;
    let press_a = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1);
    let press_b = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1);
    let release_a = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0);

    source.translate_for_device(keyboard_a, press_a);
    source.translate_for_device(keyboard_b, press_b);
    source.translate_for_device(keyboard_a, release_a);

    assert!(source.keys_held(), "keyboard B still holds keycode 30");
    assert_eq!(source.pressed[keyboard_b], HashSet::from([30]));
    source
        .reset_keyboard_state_for_devices(&[keyboard_a])
        .unwrap();
    assert!(source.keys_held(), "disconnecting A must not clear B");
}

#[test]
fn surviving_keyboard_modifiers_are_replayed_after_disconnect() {
    let keyboard_a = 0;
    let keyboard_b = 1;
    // Linux evdev keycodes: Left Ctrl, Left Shift, Left Alt, Right Alt.
    let modifiers = [
        (29, Some(&["Control", "Ctrl"][..])),
        (42, Some(&["Shift"][..])),
        (56, Some(&["Mod1", "Alt"][..])),
        // Right Alt / AltGr is layout-dependent: it may be a real Alt
        // modifier or the virtual LevelThree modifier.
        (
            100,
            Some(&["Mod1", "Alt", "Mod5", "LevelThree", "AltGr"][..]),
        ),
    ];

    for (keycode, names) in modifiers {
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: test_state(),
            layout_provider: Box::new(TestLayoutProvider),
            layout_generation: Some(1),
            layout_available: true,
            raw_key_events_total: 0,
            layout_unavailable_key_events_total: 0,
            pending: VecDeque::new(),
            raw_events: Vec::new(),
            raw_event_cursor: 0,
            raw_event_device: None,
            pending_lost_devices: Vec::new(),
            pressed: Vec::new(),
            last_device_refresh: Instant::now(),
            wake: None,
            topology_watch: None,
            topology_refresh_pending: false,
        };
        source.translate_for_device(
            keyboard_b,
            evdev::InputEvent::new(evdev::EventType::KEY.0, keycode, 1),
        );
        // A different keyboard has a key held when it disconnects. Its
        // removal must not affect the modifier held on keyboard B.
        source.translate_for_device(
            keyboard_a,
            evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1),
        );
        source.pending.clear();
        source
            .reset_keyboard_state_for_devices(&[keyboard_a])
            .unwrap();

        assert!(
            names.unwrap().iter().any(|name| {
                source
                    .state
                    .mod_name_is_active(
                        name,
                        xkbcommon_rs::xkb_state::StateComponent::MODS_EFFECTIVE,
                    )
                    .unwrap_or(false)
            }),
            "modifier keycode {keycode} was not restored"
        );

        source.translate_for_device(
            keyboard_b,
            evdev::InputEvent::new(evdev::EventType::KEY.0, keycode, 0),
        );
        assert!(
            names.unwrap().iter().all(|name| {
                !source
                    .state
                    .mod_name_is_active(
                        name,
                        xkbcommon_rs::xkb_state::StateComponent::MODS_EFFECTIVE,
                    )
                    .unwrap_or(false)
            }),
            "modifier keycode {keycode} remained active after release"
        );
        assert!(!source.pressed[keyboard_b].contains(&u32::from(keycode)));
    }
}

#[test]
fn quiet_period_rejects_already_queued_input() {
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::from([InputEvent::Text("a".into())]),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };

    assert!(!source
        .wait_for_input_quiet(Duration::from_millis(40))
        .unwrap());
}

#[test]
fn auto_repeat_does_not_clear_the_held_key() {
    let mut state = test_state();
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: std::mem::replace(&mut state, test_state()),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
    // Value 2 is kernel auto-repeat. It must not be mistaken for a release.
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 2));
    assert!(source.keys_held());
}

#[test]
fn disconnect_resets_held_keys_and_modifier_state() {
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 29, 1));
    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
    assert!(source.keys_held());
    source.reset_keyboard_state().unwrap();
    assert!(!source.keys_held());
    assert_eq!(source.pending.pop_back(), Some(InputEvent::Reset));
    source.pending.clear();

    source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
    assert!(matches!(
        source.pending.pop_front(),
        Some(InputEvent::Key(_))
    ));
    assert_eq!(source.pending.pop_front(), Some(InputEvent::TextChar('a')));
}

#[test]
fn held_key_at_release_deadline_is_rejected() {
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: vec![HashSet::from([30])],
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    let error = source.wait_for_key_release(Duration::ZERO).unwrap_err();
    assert!(error.message.contains("timed out"));
}

#[test]
fn empty_device_set_is_not_treated_as_a_successful_poll() {
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };
    assert!(matches!(
        source.poll_once(Duration::ZERO, false),
        Err(EvdevError::AllDevicesLost)
    ));
}

#[test]
fn pending_event_overflow_resets_the_queue_and_accepts_fresh_input() {
    let mut source = EvdevSource {
        devices: Vec::new(),
        state: test_state(),
        layout_provider: Box::new(TestLayoutProvider),
        layout_generation: Some(1),
        layout_available: true,
        raw_key_events_total: 0,
        layout_unavailable_key_events_total: 0,
        pending: VecDeque::new(),
        raw_events: Vec::new(),
        raw_event_cursor: 0,
        raw_event_device: None,
        pending_lost_devices: Vec::new(),
        pressed: Vec::new(),
        last_device_refresh: Instant::now(),
        wake: None,
        topology_watch: None,
        topology_refresh_pending: false,
    };

    for _ in 0..=MAX_PENDING_EVENTS {
        source.queue_event(InputEvent::Text("a".into()));
    }

    assert_eq!(source.pending.len(), 1);
    assert_eq!(source.pending.front(), Some(&InputEvent::Reset));

    // Overflow is a synchronization boundary, not a permanent failure:
    // newer input must follow the reset so the daemon can resume matching.
    source.queue_event(InputEvent::Text("recovered".into()));
    assert_eq!(
        source.pending,
        VecDeque::from([InputEvent::Reset, InputEvent::Text("recovered".into())])
    );
}
