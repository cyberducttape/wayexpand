//! Optional Fcitx5 direct-commit bridge.
//!
//! This client speaks a deliberately small D-Bus contract so the daemon can
//! use an Fcitx5 input context's exact surrounding text without observing
//! raw application contents itself. The bridge is optional: it is discovered
//! with a bounded `BuildId` call and is never started or installed by the
//! daemon.

use std::time::Duration;

use futures_lite::future::{block_on, race};
use zbus::{proxy::MethodFlags, Connection, Proxy};

const SERVICE: &str = "org.fcitx.Fcitx5";
const OBJECT: &str = "/io/github/silouanwright/SnipExpand";
const INTERFACE: &str = "io.github.silouanwright.SnipExpand.Fcitx5";
const METHOD_TIMEOUT: Duration = Duration::from_millis(500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const RETRY_DELAY: Duration = Duration::from_millis(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceResult {
    Committed,
    /// The bridge definitely did not alter the target. The ordinary libei
    /// path may be used only for statuses where the bridge was inapplicable.
    FallbackAllowed(&'static str),
    /// The bridge refused this expansion (selection, mismatch, sensitive
    /// field, or an unknown status). Never issue raw backspaces.
    Blocked(String),
    /// D-Bus transport failed; the bridge may have altered the target. Never
    /// issue raw backspaces, and recreate the route rather than refusing every
    /// later expansion against a bridge that may have gone away.
    TransportFailed(String),
}

/// Fcitx5 bridge status values. The compatible bridge returns these as `u32`.
const COMMITTED: u32 = 0;
const INVALID_REQUEST: u32 = 1;
const NO_FOCUS: u32 = 2;
const NO_SURROUNDING_TEXT: u32 = 3;
const SELECTION: u32 = 4;
const TRIGGER_MISMATCH: u32 = 5;
const PASSWORD_FIELD: u32 = 6;
const SENSITIVE_HINT_SUPPRESSED: u32 = 7;

pub struct Client {
    connection: Connection,
}

impl Client {
    /// Connect and verify that the bridge object is present. Any unavailable
    /// service is treated as an ordinary optional-feature miss.
    pub fn connect() -> Option<Self> {
        let connection = block_on(race(
            async {
                let builder = zbus::connection::Builder::session()
                    .ok()?
                    .method_timeout(METHOD_TIMEOUT);
                builder.build().await.ok()
            },
            async {
                async_io::Timer::after(CONNECT_TIMEOUT).await;
                None
            },
        ))?;
        let client = Self { connection };
        match block_on(client.build_id()) {
            Ok(_) => Some(client),
            Err(_) => None,
        }
    }

    /// Attempt exact surrounding-text replacement, then the bridge's
    /// short-lived input-context-bound fallback if surrounding text is not
    /// available. A refusal involving a selection, mismatch, or sensitive
    /// field is intentionally terminal for this expansion.
    pub fn replace(&self, expected: &str, replacement: &str) -> ReplaceResult {
        match block_on(self.call("ReplaceWithPolicy", expected, replacement)) {
            Ok(COMMITTED) => ReplaceResult::Committed,
            Ok(NO_SURROUNDING_TEXT) => {
                std::thread::sleep(RETRY_DELAY);
                match block_on(self.call(
                    "ReplaceWithoutSurroundingWithPolicy",
                    expected,
                    replacement,
                )) {
                    Ok(COMMITTED) => ReplaceResult::Committed,
                    Ok(status) => classify_status(status),
                    Err(error) => ReplaceResult::TransportFailed(error),
                }
            }
            Ok(status) => classify_status(status),
            Err(error) => ReplaceResult::TransportFailed(error),
        }
    }

    async fn build_id(&self) -> std::result::Result<String, String> {
        self.call_build_id().await
    }

    async fn call_build_id(&self) -> std::result::Result<String, String> {
        let proxy = Proxy::new(&self.connection, SERVICE, OBJECT, INTERFACE)
            .await
            .map_err(|error| error.to_string())?;
        proxy
            .call_with_flags("BuildId", MethodFlags::NoAutoStart.into(), &())
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Fcitx5 bridge returned no BuildId reply".into())
    }

    async fn call(
        &self,
        method: &str,
        expected: &str,
        replacement: &str,
    ) -> std::result::Result<u32, String> {
        let proxy = Proxy::new(&self.connection, SERVICE, OBJECT, INTERFACE)
            .await
            .map_err(|error| error.to_string())?;
        proxy
            .call_with_flags(
                method,
                MethodFlags::NoAutoStart.into(),
                &(expected, replacement, false),
            )
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Fcitx5 bridge returned no replacement status".into())
    }
}

fn classify_status(status: u32) -> ReplaceResult {
    match status {
        INVALID_REQUEST => ReplaceResult::Blocked("Fcitx5 bridge rejected the request".into()),
        // No focused Fcitx context means the application is outside the
        // bridge's ownership boundary; retain the existing libei behavior.
        NO_FOCUS => ReplaceResult::FallbackAllowed("Fcitx5 has no focused input context"),
        NO_SURROUNDING_TEXT => {
            ReplaceResult::Blocked("Fcitx5 surrounding text remained unavailable".into())
        }
        SELECTION => ReplaceResult::Blocked("Fcitx5 reported an active selection".into()),
        TRIGGER_MISMATCH => {
            ReplaceResult::Blocked("Fcitx5 surrounding text did not match the trigger".into())
        }
        PASSWORD_FIELD => ReplaceResult::Blocked("Fcitx5 reported a password field".into()),
        SENSITIVE_HINT_SUPPRESSED => {
            ReplaceResult::Blocked("Fcitx5 suppressed the sensitive-field request".into())
        }
        other => ReplaceResult::Blocked(format!("Fcitx5 bridge returned unknown status {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_no_focus_allows_raw_route_fallback() {
        assert!(matches!(
            classify_status(NO_FOCUS),
            ReplaceResult::FallbackAllowed(_)
        ));
        assert!(matches!(
            classify_status(PASSWORD_FIELD),
            ReplaceResult::Blocked(_)
        ));
        assert!(matches!(classify_status(99), ReplaceResult::Blocked(_)));
    }
}
