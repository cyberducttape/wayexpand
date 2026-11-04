//! Bounded polling, keyboard-device lifecycle, and event-queue management.

use super::*;

impl EvdevSource {
    pub(super) fn poll_once(
        &mut self,
        timeout: Duration,
        with_wake: bool,
    ) -> Result<bool, EvdevError> {
        if self.devices.is_empty() {
            return Err(EvdevError::AllDevicesLost);
        }
        // Finish a retained device batch before polling again or changing
        // device slots. This bounds translation work per turn without
        // dropping events from the kernel's already-drained batch.
        if self.has_buffered_raw_events() {
            let (key_activity, _) = self.process_buffered_raw_events(MAX_RAW_EVENTS_PER_SLICE);
            if !self.has_buffered_raw_events() {
                self.apply_pending_lost_devices()?;
            }
            return Ok(key_activity);
        }
        self.apply_pending_lost_devices()?;
        if self.devices.is_empty() {
            return Err(EvdevError::AllDevicesLost);
        }
        POLL_BUFFERS.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            scratch.descriptors.clear();
            scratch.ready.clear();
            scratch.lost.clear();
            scratch
                .descriptors
                .extend(self.devices.iter().map(|device| libc::pollfd {
                    fd: device.as_fd().as_raw_fd(),
                    events: libc::POLLIN | libc::POLLERR | libc::POLLHUP | libc::POLLNVAL,
                    revents: 0,
                }));
            let device_count = scratch.descriptors.len();
            let topology_index = self.topology_watch.as_ref().map(|watch| {
                scratch.descriptors.push(libc::pollfd {
                    fd: watch.as_raw_fd(),
                    events: libc::POLLIN | libc::POLLERR | libc::POLLHUP | libc::POLLNVAL,
                    revents: 0,
                });
                scratch.descriptors.len() - 1
            });
            let wake = with_wake.then_some(self.wake.as_deref()).flatten();
            if let Some(wake) = wake {
                scratch.descriptors.push(libc::pollfd {
                    fd: wake.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
            let timeout = libc::timespec {
                tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
                tv_nsec: timeout.subsec_nanos().into(),
            };
            // SAFETY: descriptors contain live borrowed fds owned by `self`
            // for the duration of this call; ppoll does not retain pointers.
            let poll_result = unsafe {
                libc::ppoll(
                    scratch.descriptors.as_mut_ptr(),
                    scratch.descriptors.len() as libc::nfds_t,
                    &timeout,
                    std::ptr::null(),
                )
            };
            if poll_result < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    return Ok(false);
                }
                return Err(EvdevError::Poll(error.to_string()));
            }
            if wake.is_some()
                && scratch
                    .descriptors
                    .last()
                    .is_some_and(|fd| fd.revents & libc::POLLIN != 0)
            {
                if let Some(wake) = wake {
                    drain_wake_fd(wake.as_fd());
                }
            }
            let topology_revents = topology_index.map(|index| scratch.descriptors[index].revents);
            for index in 0..device_count {
                let revents = scratch.descriptors[index].revents;
                if revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    scratch.lost.push(index);
                } else if revents & libc::POLLIN != 0 {
                    scratch.ready.push(index);
                }
            }
            let topology_changed = match topology_revents {
                Some(revents)
                    if revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 =>
                {
                    return Err(EvdevError::HotplugMonitor(
                        "inotify descriptor became unavailable".into(),
                    ));
                }
                Some(revents) if revents & libc::POLLIN != 0 => {
                    let watch = self.topology_watch.as_ref().ok_or_else(|| {
                        EvdevError::HotplugMonitor("inotify descriptor disappeared".into())
                    })?;
                    drain_topology_events(watch.as_fd())?
                }
                _ => false,
            };
            self.pending_lost_devices
                .extend(scratch.lost.iter().copied());
            let key_activity = self.drain_ready(&scratch.ready)?;
            if !self.has_buffered_raw_events() {
                self.apply_pending_lost_devices()?;
            }
            self.topology_refresh_pending |= topology_changed;
            Ok(key_activity || topology_changed)
        })
    }

    /// Previous allocating poll loop retained only in the benchmark feature
    /// to compare the same capture/translation workload before and after the
    /// reusable-buffer change.
    #[cfg(feature = "benchmark")]
    pub(super) fn poll_once_allocating(
        &mut self,
        timeout: Duration,
        with_wake: bool,
    ) -> Result<bool, EvdevError> {
        if self.devices.is_empty() {
            return Err(EvdevError::AllDevicesLost);
        }
        self.apply_pending_lost_devices()?;
        if self.devices.is_empty() {
            return Err(EvdevError::AllDevicesLost);
        }
        let borrowed: Vec<BorrowedFd<'_>> =
            self.devices.iter().map(|device| device.as_fd()).collect();
        let mut fds: Vec<rustix::event::PollFd<'_>> = borrowed
            .iter()
            .map(|fd| {
                rustix::event::PollFd::new(
                    fd,
                    rustix::event::PollFlags::IN
                        | rustix::event::PollFlags::ERR
                        | rustix::event::PollFlags::HUP
                        | rustix::event::PollFlags::NVAL,
                )
            })
            .collect();
        let device_count = fds.len();
        let topology_index = self.topology_watch.as_ref().map(|watch| {
            fds.push(rustix::event::PollFd::new(
                watch,
                rustix::event::PollFlags::IN
                    | rustix::event::PollFlags::ERR
                    | rustix::event::PollFlags::HUP
                    | rustix::event::PollFlags::NVAL,
            ));
            fds.len() - 1
        });
        let wake = with_wake.then_some(self.wake.as_deref()).flatten();
        if let Some(wake) = wake {
            fds.push(rustix::event::PollFd::new(
                wake,
                rustix::event::PollFlags::IN,
            ));
        }
        poll_with_wake(&mut fds, timeout, wake)?;
        if wake.is_some() {
            fds.pop();
        }
        let topology_revents = topology_index.map(|index| fds[index].revents());
        let mut ready = Vec::new();
        let mut lost = Vec::new();
        for (index, fd) in fds.iter().take(device_count).enumerate() {
            let revents = fd.revents();
            if revents.intersects(
                rustix::event::PollFlags::ERR
                    | rustix::event::PollFlags::HUP
                    | rustix::event::PollFlags::NVAL,
            ) {
                lost.push(index);
            } else if revents.contains(rustix::event::PollFlags::IN) {
                ready.push(index);
            }
        }
        drop(fds);
        drop(borrowed);
        let topology_changed = match topology_revents {
            Some(revents)
                if revents.intersects(
                    rustix::event::PollFlags::ERR
                        | rustix::event::PollFlags::HUP
                        | rustix::event::PollFlags::NVAL,
                ) =>
            {
                return Err(EvdevError::HotplugMonitor(
                    "inotify descriptor became unavailable".into(),
                ));
            }
            Some(revents) if revents.contains(rustix::event::PollFlags::IN) => {
                let watch = self.topology_watch.as_ref().ok_or_else(|| {
                    EvdevError::HotplugMonitor("inotify descriptor disappeared".into())
                })?;
                drain_topology_events(watch.as_fd())?
            }
            _ => false,
        };
        self.pending_lost_devices.extend(lost);
        let key_activity = self.drain_ready(&ready)?;
        if !self.has_buffered_raw_events() {
            self.apply_pending_lost_devices()?;
        }
        self.topology_refresh_pending |= topology_changed;
        Ok(key_activity || topology_changed)
    }

    #[cfg(test)]
    pub(super) fn reset_keyboard_state(&mut self) -> Result<(), EvdevError> {
        let context = Context::new(0).map_err(|error| EvdevError::Keymap(error.to_string()))?;
        let keymap = Keymap::new_from_names(context, None, 0)
            .map_err(|error| EvdevError::Keymap(error.to_string()))?;
        self.state = State::new(keymap);
        self.pressed.clear();
        self.queue_event(InputEvent::Reset);
        tracing::warn!("resetting evdev keyboard state after device disconnect");
        Ok(())
    }

    pub(super) fn reset_keyboard_state_for_devices(
        &mut self,
        lost_devices: &[usize],
    ) -> Result<(), EvdevError> {
        for &index in lost_devices {
            if let Some(pressed) = self.pressed.get_mut(index) {
                pressed.clear();
            }
        }
        // XKB state is kept global for cross-device modifier/chord handling.
        // Rebuild it after a disconnect because the release events for that
        // device can no longer arrive; held-key ownership itself remains
        // per-device above.
        let context = Context::new(0).map_err(|error| EvdevError::Keymap(error.to_string()))?;
        let keymap = Keymap::new_from_names(context, None, 0)
            .map_err(|error| EvdevError::Keymap(error.to_string()))?;
        self.state = State::new(keymap);
        // Reconstruct the global XKB state from every key that is still held
        // on a surviving device. Resetting without replaying these keys makes
        // `pressed` report modifiers that XKB no longer knows about, so the
        // next shortcut is interpreted with two different keyboard states.
        let mut held_keys = self
            .pressed
            .iter()
            .flat_map(|keys| keys.iter().copied())
            .collect::<Vec<_>>();
        held_keys.sort_unstable();
        for keycode in held_keys {
            if let Some(xkb_keycode) = keycode.checked_add(8) {
                self.state.update_key(xkb_keycode, KeyDirection::Down);
            }
        }
        self.queue_event(InputEvent::Reset);
        tracing::warn!(
            count = lost_devices.len(),
            "resetting evdev keyboard state after device disconnect"
        );
        Ok(())
    }

    /// Keep translated input bounded while the daemon is waiting for a safe
    /// replacement point. Dropping only the newest event could leave the
    /// matcher/application streams out of sync, so overflow abandons the
    /// pending transaction and emits one reset boundary instead.
    pub(super) fn queue_event(&mut self, event: InputEvent) {
        if self.pending.len() >= MAX_PENDING_EVENTS {
            INPUT_QUEUE_OVERFLOWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.pending.clear();
            self.pending.push_back(InputEvent::Reset);
            tracing::warn!(
                maximum = MAX_PENDING_EVENTS,
                "evdev input queue overflow; resetting matcher state"
            );
            return;
        }
        self.pending.push_back(event);
    }

    pub(super) fn refresh_devices(&mut self) {
        let discovery = device::discover_keyboards();
        for keyboard in discovery.keyboards {
            if self
                .devices
                .iter()
                .any(|existing| existing.path() == keyboard.path())
            {
                continue;
            }
            tracing::info!(path = %keyboard.path().display(), "keyboard device connected");
            self.devices.push(Box::new(keyboard));
            self.pressed.push(HashSet::new());
        }
    }

    pub(super) fn drain_ready(&mut self, ready_indices: &[usize]) -> Result<bool, EvdevError> {
        let mut key_activity = false;
        let mut remaining = MAX_RAW_EVENTS_PER_SLICE;
        for (ready_position, &index) in ready_indices.iter().enumerate() {
            if remaining == 0 {
                // More ready descriptors remain unread. The output monitor
                // must conservatively interrupt rather than miss a physical
                // key that is still waiting on another device.
                key_activity = true;
                break;
            }
            let read_result = {
                let (devices, raw_events) = (&mut self.devices, &mut self.raw_events);
                devices[index].fetch_events(raw_events)
            };
            if let Err(error) = read_result {
                return Err(EvdevError::Read {
                    path: self.devices[index].path().display().to_string(),
                    message: error.to_string(),
                });
            }
            self.raw_event_cursor = 0;
            self.raw_event_device = Some(index);
            let (activity, processed) = self.process_buffered_raw_events(remaining);
            key_activity |= activity;
            remaining -= processed;
            if self.has_buffered_raw_events() {
                // The retained remainder has not yet been inspected for key
                // activity. Treat it as an interruption until the next
                // bounded slice classifies it.
                key_activity = true;
                break;
            }
            if remaining == 0 {
                if ready_position + 1 < ready_indices.len() {
                    key_activity = true;
                }
                break;
            }
        }
        Ok(key_activity)
    }

    pub(super) fn has_buffered_raw_events(&self) -> bool {
        self.raw_event_cursor < self.raw_events.len()
    }

    pub(super) fn process_buffered_raw_events(&mut self, limit: usize) -> (bool, usize) {
        let Some(device_index) = self.raw_event_device else {
            return (false, 0);
        };
        let start = self.raw_event_cursor;
        let end = start.saturating_add(limit).min(self.raw_events.len());
        let mut key_activity = false;
        while self.raw_event_cursor < end {
            let event = self.raw_events[self.raw_event_cursor];
            self.raw_event_cursor += 1;
            key_activity |= matches!(event.destructure(), evdev::EventSummary::Key(..));
            self.translate_for_device(device_index, event);
        }
        if self.raw_event_cursor == self.raw_events.len() {
            self.raw_events.clear();
            self.raw_event_cursor = 0;
            self.raw_event_device = None;
        }
        (key_activity, end - start)
    }

    pub(super) fn apply_pending_lost_devices(&mut self) -> Result<(), EvdevError> {
        if self.pending_lost_devices.is_empty() {
            return Ok(());
        }
        self.pending_lost_devices.sort_unstable();
        self.pending_lost_devices.dedup();
        let lost = std::mem::take(&mut self.pending_lost_devices);
        for &index in lost.iter().rev() {
            if let Some(device) = self.devices.get(index) {
                tracing::warn!(path = %device.path().display(), "keyboard device disconnected");
            }
        }
        self.reset_keyboard_state_for_devices(&lost)?;
        for index in lost.into_iter().rev() {
            if index < self.devices.len() {
                self.devices.remove(index);
                self.pressed.remove(index);
            }
        }
        Ok(())
    }
}
