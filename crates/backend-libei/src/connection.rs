use super::{
    read_portal_token_if_enabled, store_portal_token_if_enabled, ConnectCancellation, LibeiError,
    LibeiOptions,
};
use std::{os::unix::net::UnixStream, sync::OnceLock, time::Duration};

/// One Tokio runtime for every portal session in the process.
///
/// ashpd caches a single D-Bus connection process-wide, and zbus runs that
/// connection's reader on the runtime that was current when it was opened.
/// A runtime per session therefore broke every later session: once the first
/// was closed its runtime stopped, the cached connection had no reader, and a
/// reconnect waited for portal replies until it timed out and the daemon
/// exited. A runtime that lives as long as the process keeps the connection
/// served for reconnects.
fn portal_runtime() -> Result<&'static tokio::runtime::Runtime, LibeiError> {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .thread_name("wayexpand-portal")
        .enable_all()
        .build()
        .map_err(|error| LibeiError::Portal(error.to_string()))?;
    // A concurrent first caller may have won; its runtime is used and this
    // one is dropped unused.
    Ok(RUNTIME.get_or_init(|| runtime))
}

/// Keeps a RemoteDesktop portal session open, and closes it when dropped.
pub(super) struct PortalKeepalive {
    proxy: Option<ashpd::desktop::remote_desktop::RemoteDesktop>,
    session: Option<ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>>,
}

impl PortalKeepalive {
    /// Close the portal session, waiting a bounded time for the portal.
    pub(super) fn close(self) {
        drop(self);
    }
}

impl Drop for PortalKeepalive {
    /// A session that is only dropped would stay open on the shared D-Bus
    /// connection, and the portal may refuse or prompt for the next one, so
    /// every exit path (including a failed handshake) closes it.
    fn drop(&mut self) {
        let proxy = self.proxy.take();
        let Some(session) = self.session.take() else {
            return;
        };
        let Ok(runtime) = portal_runtime() else {
            return;
        };
        let closing = runtime.spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            drop(proxy);
        });
        // Wait for the close unless this thread is already inside a Tokio
        // runtime, where blocking would panic; the task then finishes on its
        // own.
        if tokio::runtime::Handle::try_current().is_err() {
            let _ = runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(3), closing).await });
        }
    }
}

pub(super) fn connect_portal(
    options: LibeiOptions,
    cancellation: &ConnectCancellation,
) -> Result<(UnixStream, Option<PortalKeepalive>), LibeiError> {
    use ashpd::desktop::{
        remote_desktop::{
            ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
        },
        PersistMode,
    };

    let (stream, keepalive) = portal_runtime()?.block_on(async {
        let attempt = tokio::time::timeout(Duration::from_secs(30), async {
            let proxy: RemoteDesktop = RemoteDesktop::new()
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            // Owned by a keepalive from here on, so every later failure, and
            // the timeout dropping this future, closes the session instead of
            // leaving it open on the shared D-Bus connection.
            let mut keepalive = PortalKeepalive {
                proxy: None,
                session: Some(
                    proxy
                        .create_session(Default::default())
                        .await
                        .map_err(|error| LibeiError::Portal(error.to_string()))?,
                ),
            };
            let Some(session) = keepalive.session.as_ref() else {
                return Err(LibeiError::Portal("portal session was not kept".into()));
            };

            let stored_token = read_portal_token_if_enabled(
                options.persist_portal_token,
                options.portal_token_path.as_deref(),
            )
            .map_err(|error| {
                LibeiError::Portal(format!(
                    "could not securely read portal restoration token: {error}"
                ))
            })?;
            proxy
                .select_devices(
                    session,
                    SelectDevicesOptions::default()
                        .set_devices(ashpd::enumflags2::BitFlags::from_flag(DeviceType::Keyboard))
                        .set_restore_token(stored_token.as_deref())
                        .set_persist_mode(if options.persist_portal_token {
                            PersistMode::ExplicitlyRevoked
                        } else {
                            PersistMode::DoNot
                        }),
                )
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            let start_response = proxy
                .start(session, None, StartOptions::default())
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?
                .response()
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            if let Some(token) = start_response.restore_token() {
                store_portal_token_if_enabled(
                    options.persist_portal_token,
                    options.portal_token_path.as_deref(),
                    token,
                )
                .map_err(|error| {
                    LibeiError::Portal(format!(
                        "could not securely store portal restoration token: {error}"
                    ))
                })?;
            }

            let fd = proxy
                .connect_to_eis(session, ConnectToEISOptions::default())
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            keepalive.proxy = Some(proxy);
            Ok::<_, LibeiError>((UnixStream::from(fd), keepalive))
        });
        futures_lite::future::race(
            async {
                attempt.await.map_err(|_| {
                    LibeiError::Portal("portal session timed out after 30 seconds".into())
                })?
            },
            async {
                cancellation.cancelled().await;
                Err(LibeiError::Cancelled)
            },
        )
        .await
    })?;

    Ok((stream, Some(keepalive)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn every_portal_session_shares_one_runtime() {
        // A later session must reach the cached D-Bus connection's reader,
        // which runs on the runtime that opened it.
        let first = super::portal_runtime().unwrap() as *const tokio::runtime::Runtime;
        let second = super::portal_runtime().unwrap() as *const tokio::runtime::Runtime;
        assert_eq!(first, second);
    }

    #[test]
    fn cancelling_a_pending_portal_attempt_returns_promptly() {
        let cancellation = super::ConnectCancellation::default();
        let worker_cancellation = cancellation.clone();
        let cancel_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            worker_cancellation.cancel();
        });
        let runtime = super::portal_runtime().unwrap();
        let result = runtime.block_on(futures_lite::future::race(
            async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, super::LibeiError>(())
            },
            async {
                cancellation.cancelled().await;
                Err(super::LibeiError::Cancelled)
            },
        ));
        cancel_thread.join().unwrap();
        assert!(matches!(result, Err(super::LibeiError::Cancelled)));
    }
}
