//! Transactional management of GNOME's IBus input source.
use anyhow::{bail, Context, Result};
use wayexpand_backend_ibus::gnome_input_sources as variant;

const SCHEMA: &str = "org.gnome.desktop.input-sources";

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    sources: Vec<variant::InputSource>,
    current: u32,
}

impl Snapshot {
    pub(crate) fn capture() -> Result<Self> {
        let sources_raw = get("sources").context("could not read GNOME input sources")?;
        let current_raw = get("current").context("could not read GNOME active input source")?;
        let sources = variant::parse_sources(&sources_raw)
            .map_err(anyhow::Error::msg)
            .context("GNOME sources is not a valid a(ss) tuple array")?;
        let current = variant::parse_current(&current_raw)
            .map_err(anyhow::Error::msg)
            .context("GNOME current is not a valid uint32")?;
        if !sources.is_empty() && current as usize >= sources.len() {
            bail!("GNOME current input-source index is outside the configured source list");
        }
        Ok(Self { sources, current })
    }

    pub(crate) fn register_and_activate(&self) -> Result<()> {
        // Re-read before writing so a concurrent user change is retained.
        let raw = get("sources").context("could not reread GNOME input sources")?;
        let mut sources = variant::parse_sources(&raw)
            .map_err(anyhow::Error::msg)
            .context("GNOME sources is not a valid a(ss) tuple array")?;
        let previous_len = sources.len();
        variant::ensure_wayexpand_source(&mut sources);
        if sources.len() != previous_len {
            set("sources", &variant::format_sources(&sources))?;
            sources = variant::parse_sources(
                &get("sources").context("could not verify GNOME input-source registration")?,
            )
            .map_err(anyhow::Error::msg)
            .context("GNOME returned an invalid input-source list after registration")?;
        }
        let Some(index) = variant::wayexpand_index(&sources) else {
            bail!("GNOME did not retain the WayExpand IBus input source");
        };
        if index >= sources.len() {
            bail!("GNOME WayExpand source index is invalid");
        }
        set("current", &format!("{index}"))?;
        let active = variant::parse_current(
            &get("current").context("could not verify active GNOME input source")?,
        )
        .map_err(anyhow::Error::msg)
        .context("GNOME returned an invalid active input-source index")?
            as usize;
        let confirmed = variant::parse_sources(
            &get("sources").context("could not verify GNOME input-source list")?,
        )
        .map_err(anyhow::Error::msg)
        .context("GNOME returned an invalid input-source list")?;
        if confirmed
            .get(active)
            .is_none_or(|source| source.kind != "ibus" || source.id != "wayexpand")
        {
            bail!("GNOME did not activate WayExpand as the current input source");
        }
        Ok(())
    }

    pub(crate) fn restore_with(&self, mut write: impl FnMut(&str, &str) -> bool) -> Result<()> {
        let sources = variant::format_sources(&self.sources);
        if !write("sources", &sources) {
            bail!("could not restore GNOME input-source list");
        }
        if !write("current", &self.current.to_string()) {
            bail!("could not restore GNOME active input-source index");
        }
        let actual_sources = variant::parse_sources(
            &get("sources").context("could not verify restored GNOME input sources")?,
        )
        .map_err(anyhow::Error::msg)?;
        let actual_current = variant::parse_current(
            &get("current").context("could not verify restored GNOME current source")?,
        )
        .map_err(anyhow::Error::msg)?;
        if actual_sources != self.sources || actual_current != self.current {
            bail!("GNOME input-source settings did not return to their previous values");
        }
        Ok(())
    }
}

fn get(key: &str) -> Result<String> {
    crate::setup_ibus::command_output("gsettings", &["get", SCHEMA, key])
        .map(|value| value.trim().to_owned())
        .ok_or_else(|| anyhow::anyhow!("gsettings get {key} failed or timed out"))
}

fn set(key: &str, value: &str) -> Result<()> {
    let status = crate::setup::run_setup_command("gsettings", &["set", SCHEMA, key, value])
        .with_context(|| format!("could not set GNOME input-source {key}"))?;
    if !status.success() {
        bail!("gsettings rejected GNOME input-source {key}");
    }
    Ok(())
}
