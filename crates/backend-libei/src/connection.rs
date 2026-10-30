use super::{
    read_portal_token_if_enabled, store_portal_token_if_enabled, ConnectCancellation, LibeiError,
    LibeiOptions,
};
use std::{
    future::Future, os::unix::net::UnixStream, path::PathBuf, sync::OnceLock, time::Duration,
};

const EIS_SOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Connect to an explicitly configured EIS socket without blocking the owner
/// thread in a synchronous `connect(2)`. Test and embedded routes use this
/// path instead of the desktop portal, so they must observe the same
/// cancellation and deadline contract.
pub(super) fn connect_socket_with_cancellation(
    path: PathBuf,
    cancellation: &ConnectCancellation,
) -> Result<UnixStream, LibeiError> {
    if cancellation.is_cancelled() {
        return Err(LibeiError::Cancelled);
    }
    let stream = portal_runtime()?.block_on(connect_stream_with_cancellation(
        tokio::net::UnixStream::connect(path),
        cancellation,
    ))?;
    stream.into_std().map_err(LibeiError::Connect)
}

async fn connect_stream_with_cancellation<F>(
    connect: F,
    cancellation: &ConnectCancellation,
) -> Result<tokio::net::UnixStream, LibeiError>
where
    F: Future<Output = std::io::Result<tokio::net::UnixStream>>,
{
    race_cancellation(
        async {
            tokio::time::timeout(EIS_SOCKET_CONNECT_TIMEOUT, connect)
                .await
                .map_err(|_| {
                    LibeiError::Connect(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "EIS socket connection exceeded its deadline",
                    ))
                })?
                .map_err(LibeiError::Connect)
        },
        cancellation,
    )
    .await
}

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

async fn race_cancellation<T>(
    operation: impl Future<Output = Result<T, LibeiError>>,
    cancellation: &ConnectCancellation,
) -> Result<T, LibeiError> {
    futures_lite::future::race(operation, async {
        cancellation.cancelled().await;
        Err(LibeiError::Cancelled)
    })
    .await
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
        race_cancellation(
            async {
                attempt.await.map_err(|_| {
                    LibeiError::Portal("portal session timed out after 30 seconds".into())
                })?
            },
            cancellation,
        )
        .await
    })?;

    Ok((stream, Some(keepalive)))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let started = std::time::Instant::now();
        let cancellation = super::ConnectCancellation::default();
        let worker_cancellation = cancellation.clone();
        let cancel_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            worker_cancellation.cancel();
        });
        let runtime = super::portal_runtime().unwrap();
        let result = runtime.block_on(super::race_cancellation(
            async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, super::LibeiError>(())
            },
            &cancellation,
        ));
        cancel_thread.join().unwrap();
        assert!(matches!(result, Err(super::LibeiError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn direct_socket_connect_observes_preexisting_cancellation() {
        let cancellation = super::ConnectCancellation::default();
        cancellation.cancel();
        let started = std::time::Instant::now();
        let error = connect_socket_with_cancellation(
            PathBuf::from("/path/that/must/not/be/connected"),
            &cancellation,
        )
        .unwrap_err();
        assert!(matches!(error, LibeiError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn direct_socket_connect_cancels_and_drops_a_pending_attempt() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let cancellation = super::ConnectCancellation::default();
        let worker_cancellation = cancellation.clone();
        let cancel_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            worker_cancellation.cancel();
        });
        let dropped = Arc::new(AtomicBool::new(false));
        let connect_dropped = Arc::clone(&dropped);
        let pending_connect = async move {
            let _drop_signal = DropSignal(connect_dropped);
            std::future::pending::<std::io::Result<tokio::net::UnixStream>>().await
        };
        let started = std::time::Instant::now();
        let result =
            super::portal_runtime()
                .unwrap()
                .block_on(super::connect_stream_with_cancellation(
                    pending_connect,
                    &cancellation,
                ));
        cancel_thread.join().unwrap();

        assert!(matches!(result, Err(LibeiError::Cancelled)));
        assert!(
            dropped.load(Ordering::Acquire),
            "pending connect future survived cancellation"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn direct_socket_connect_reports_missing_socket() {
        let error = connect_socket_with_cancellation(
            PathBuf::from("/dev/null/wayexpand-no-eis-socket"),
            &ConnectCancellation::default(),
        )
        .unwrap_err();
        assert!(matches!(error, LibeiError::Connect(_)));
    }

    #[test]
    fn direct_socket_connect_returns_a_standard_unix_stream() {
        use std::os::unix::net::UnixListener;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wayexpand-eis-connect-{}-{nonce}.sock",
            std::process::id()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let accept = std::thread::spawn(move || listener.accept().unwrap().0);

        let stream =
            connect_socket_with_cancellation(path.clone(), &ConnectCancellation::default())
                .unwrap();
        assert!(stream.peer_addr().is_ok());
        let accepted = accept.join().unwrap();
        assert!(accepted.peer_addr().is_ok());
        std::fs::remove_file(path).unwrap();
    }
}
