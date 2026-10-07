use super::{
    read_portal_token_if_enabled, store_portal_token_if_enabled, LibeiError, LibeiOptions,
};
use std::{os::unix::net::UnixStream, time::Duration};

pub(super) struct PortalKeepalive {
    pub(super) _proxy: ashpd::desktop::remote_desktop::RemoteDesktop,
    pub(super) _session: ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
    // Keep the Tokio reactor alive until the portal proxies have been dropped.
    pub(super) _runtime: tokio::runtime::Runtime,
}

impl PortalKeepalive {
    /// Close the portal session before shutting down its Tokio runtime.
    ///
    /// `Runtime::shutdown_timeout` bounds runtime teardown even when a portal
    /// implementation leaves a task pending. This keeps the lifecycle
    /// explicit while retaining a hard upper bound for the daemon.
    pub(super) fn close(self) {
        let PortalKeepalive {
            _proxy: proxy,
            _session: session,
            _runtime: runtime,
        } = self;
        // Construct the timer inside the runtime context. Constructing
        // tokio::time::timeout outside block_on() panics because no reactor
        // is current on the daemon's shutdown worker thread.
        let _ = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), session.close()).await
        });
        drop(proxy);
        runtime.shutdown_timeout(Duration::from_secs(2));
    }
}

pub(super) fn connect_portal(
    options: LibeiOptions,
) -> Result<(UnixStream, Option<PortalKeepalive>), LibeiError> {
    use ashpd::desktop::{
        remote_desktop::{
            ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
        },
        PersistMode,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| LibeiError::Portal(error.to_string()))?;
    let (stream, proxy, session) = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            let proxy: RemoteDesktop = RemoteDesktop::new()
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            let session = proxy
                .create_session(Default::default())
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

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
                    &session,
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
                .start(&session, None, StartOptions::default())
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
                .connect_to_eis(&session, ConnectToEISOptions::default())
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            Ok::<_, LibeiError>((UnixStream::from(fd), proxy, session))
        })
        .await
        .map_err(|_| LibeiError::Portal("portal session timed out after 30 seconds".into()))?
    })?;

    Ok((
        stream,
        Some(PortalKeepalive {
            _proxy: proxy,
            _session: session,
            _runtime: runtime,
        }),
    ))
}
