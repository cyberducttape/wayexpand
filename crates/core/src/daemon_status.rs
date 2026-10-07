//! Parsed view of the daemon's stable key/value control-socket status body.

use std::collections::{BTreeMap, BTreeSet};

/// The daemon's stable lifecycle state as reported by its control socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonRouteState {
    Connected,
    Reconnecting,
    Starting,
    PermissionRequired,
    PortalRevoked,
    Unsupported,
    Degraded,
    Failed,
    Stopped,
}

/// The daemon status response split into its banner and stable fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    response: String,
    fields: BTreeMap<String, String>,
    duplicate_fields: BTreeSet<String>,
}

impl DaemonStatus {
    /// Parse the daemon's line-oriented status body.
    ///
    /// Unknown fields are retained so newer daemons remain inspectable by
    /// older frontends. Lines without `key=value` are ignored after the
    /// response banner for compatibility with diagnostic banners.
    pub fn parse(response: &str) -> Self {
        let mut lines = response.lines();
        let first = lines.next().unwrap_or_default();
        let (banner, first_field) = if first.split_once('=').is_some() {
            (String::new(), Some(first))
        } else {
            (first.to_owned(), None)
        };
        let mut fields = BTreeMap::new();
        let mut duplicate_fields = BTreeSet::new();
        for (key, value) in lines
            .chain(first_field)
            .filter_map(|line| line.split_once('='))
        {
            if fields.insert(key.to_owned(), value.to_owned()).is_some() {
                duplicate_fields.insert(key.to_owned());
            }
        }
        Self {
            response: banner,
            fields,
            duplicate_fields,
        }
    }

    pub fn response(&self) -> &str {
        &self.response
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn bool_field(&self, key: &str) -> Option<bool> {
        match self.field(key)? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    pub fn u64_field(&self, key: &str) -> Option<u64> {
        self.field(key)?.parse().ok()
    }

    pub fn status_schema(&self) -> Option<u32> {
        if !self.field_is_unique("status_schema") {
            return None;
        }
        self.field("status_schema")?.parse().ok()
    }

    /// Parse the bounded set of lifecycle values understood by frontends.
    /// Unknown values remain unavailable so newer daemons fail closed in UI.
    pub fn route_state(&self) -> Option<DaemonRouteState> {
        if !self.field_is_unique("state") {
            return None;
        }
        match self.field("state")? {
            "connected" | "running" => Some(DaemonRouteState::Connected),
            "reconnecting" => Some(DaemonRouteState::Reconnecting),
            "starting" => Some(DaemonRouteState::Starting),
            "permission_required" => Some(DaemonRouteState::PermissionRequired),
            "portal_revoked" => Some(DaemonRouteState::PortalRevoked),
            "unsupported" => Some(DaemonRouteState::Unsupported),
            "degraded" => Some(DaemonRouteState::Degraded),
            "failed" => Some(DaemonRouteState::Failed),
            "stopped" => Some(DaemonRouteState::Stopped),
            _ => None,
        }
    }

    pub fn field_is_unique(&self, key: &str) -> bool {
        !self.duplicate_fields.contains(key)
    }

    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonRouteState, DaemonStatus};

    #[test]
    fn preserves_unknown_fields_and_typed_accessors() {
        let status = DaemonStatus::parse(
            "running\nstatus_schema=6\npaused=false\ncommand_queue_depth=4\nfuture=value\n",
        );
        assert_eq!(status.response(), "running");
        assert_eq!(status.status_schema(), Some(6));
        assert_eq!(status.bool_field("paused"), Some(false));
        assert_eq!(status.u64_field("command_queue_depth"), Some(4));
        assert_eq!(status.field("future"), Some("value"));
    }

    #[test]
    fn ignores_diagnostic_lines_without_a_key_value_separator() {
        let status = DaemonStatus::parse("running\nwarning: reconnecting\nstate=degraded\n");
        assert_eq!(status.field("state"), Some("degraded"));
        assert_eq!(status.route_state(), Some(DaemonRouteState::Degraded));
        assert_eq!(status.fields().count(), 1);
    }

    #[test]
    fn route_state_requires_a_known_state_field() {
        assert_eq!(
            DaemonStatus::parse("state=running\n").route_state(),
            Some(DaemonRouteState::Connected)
        );
        assert_eq!(DaemonStatus::parse("running\n").route_state(), None);
        assert_eq!(
            DaemonStatus::parse("state=future-state\n").route_state(),
            None
        );
        assert_eq!(
            DaemonStatus::parse("state=connected\nstate=failed\n").route_state(),
            None
        );
    }
}
