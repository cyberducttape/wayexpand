//! Transactional management of GNOME's IBus input source.
//!
//! The deprecated `org.gnome.desktop.input-sources current` key is ignored by
//! GNOME. Registration is stored in the supported `sources` key, while the
//! live source is read and activated through the WayExpand Shell extension's
//! GNOME `InputSourceManager` bridge.
use anyhow::{bail, Context, Result};
use wayexpand_backend_ibus::gnome_input_sources as variant;
use wayexpand_backend_ibus::gnome_input_sources::InputSource;

const SCHEMA: &str = "org.gnome.desktop.input-sources";

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    sources: Vec<InputSource>,
    active: InputSource,
}

impl Snapshot {
    pub(crate) fn capture() -> Result<Self> {
        let sources_raw = get("sources").context("could not read GNOME input sources")?;
        let sources = variant::parse_sources(&sources_raw)
            .map_err(anyhow::Error::msg)
            .context("GNOME sources is not a valid a(ss) tuple array")?;
        let active = active_source()
            .context("could not read the live GNOME Shell input source; enable or update the WayExpand GNOME Shell integration")?;
        if !sources.contains(&active) {
            bail!("GNOME Shell's active input source is missing from the configured source list");
        }
        Ok(Self { sources, active })
    }

    pub(crate) fn register_and_activate(rollback_snapshot: &mut Option<Snapshot>) -> Result<()> {
        // Capture immediately before writing. The session may have switched
        // layouts or input sources while setup was refreshing IBus.
        *rollback_snapshot = Some(
            Self::capture().context("could not capture the live GNOME source before activation")?,
        );
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
        if variant::wayexpand_index(&sources).is_none() {
            bail!("GNOME did not retain the WayExpand IBus input source");
        }

        let active = wayexpand_backend_gnome_window::activate_input_source("ibus", "wayexpand")
            .context(
                "GNOME Shell could not activate WayExpand through its live input-source manager",
            )?;
        if active != ("ibus".to_owned(), "wayexpand".to_owned()) {
            bail!(
                "GNOME Shell did not report WayExpand as the active input source after activation"
            );
        }
        let confirmed = variant::parse_sources(
            &get("sources").context("could not verify GNOME input-source registration")?,
        )
        .map_err(anyhow::Error::msg)
        .context("GNOME returned an invalid input-source list")?;
        if variant::wayexpand_index(&confirmed).is_none() {
            bail!("GNOME removed the WayExpand source during activation");
        }
        Ok(())
    }

    pub(crate) fn restore_with(
        &self,
        mut write: impl FnMut(&str, &str) -> bool,
        mut read_active: impl FnMut() -> Option<InputSource>,
        mut activate: impl FnMut(&InputSource) -> bool,
    ) -> Result<()> {
        let current_sources_raw =
            get("sources").context("could not inspect GNOME sources before rollback")?;
        let current_sources = variant::parse_sources(&current_sources_raw)
            .map_err(anyhow::Error::msg)
            .context("GNOME sources changed to an invalid value before rollback")?;
        let current_active = read_active()
            .context("could not read GNOME Shell's live input source before rollback")?;
        let (restored_sources, target_active) = restore_plan(
            &self.sources,
            &self.active,
            &current_sources,
            &current_active,
        )?;

        // Switch away before removing a source that may currently be active.
        if current_active != target_active && !activate(&target_active) {
            bail!("could not restore GNOME Shell's previous active input source");
        }
        if current_sources != restored_sources {
            let sources = variant::format_sources(&restored_sources);
            if !write("sources", &sources) {
                bail!("could not restore GNOME input-source list");
            }
        }

        let actual_sources = variant::parse_sources(
            &get("sources").context("could not verify restored GNOME input sources")?,
        )
        .map_err(anyhow::Error::msg)?;
        let actual_active =
            read_active().context("could not verify GNOME Shell's restored active input source")?;
        if actual_sources != restored_sources || actual_active != target_active {
            bail!("GNOME input-source configuration or live active source did not return to the expected state");
        }
        Ok(())
    }
}

/// Build a rollback target without overriding a different source the user
/// selected while setup was running.
fn restore_plan(
    original_sources: &[InputSource],
    original_active: &InputSource,
    current_sources: &[InputSource],
    current_active: &InputSource,
) -> Result<(Vec<InputSource>, InputSource)> {
    let mut restored_sources = current_sources.to_vec();
    if variant::wayexpand_index(original_sources).is_none() {
        if let Some(index) = variant::wayexpand_index(&restored_sources) {
            restored_sources.remove(index);
        }
    }
    let target_active = if current_active.kind != "ibus" || current_active.id != "wayexpand" {
        restored_sources
            .iter()
            .find(|source| *source == current_active)
            .cloned()
    } else {
        None
    }
    .or_else(|| {
        restored_sources
            .iter()
            .find(|source| *source == original_active)
            .cloned()
    })
    .or_else(|| restored_sources.first().cloned())
    .context("no GNOME input source remains available for rollback")?;
    Ok((restored_sources, target_active))
}

pub(crate) fn active_source() -> Result<InputSource> {
    let (kind, id) = wayexpand_backend_gnome_window::active_input_source()?;
    if kind.is_empty() || id.is_empty() {
        bail!("GNOME Shell returned an empty active input source");
    }
    Ok(InputSource { kind, id })
}

pub(crate) fn activate_source(source: &InputSource) -> Result<()> {
    let actual = wayexpand_backend_gnome_window::activate_input_source(&source.kind, &source.id)?;
    if actual != (source.kind.clone(), source.id.clone()) {
        bail!("GNOME Shell activated a different input source");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: &str, id: &str) -> InputSource {
        InputSource {
            kind: kind.into(),
            id: id.into(),
        }
    }

    #[test]
    fn rollback_restores_shell_active_source_and_removes_only_its_registration() {
        let original_sources = vec![source("xkb", "us"), source("xkb", "de")];
        let original_active = source("xkb", "de");
        let current_sources = vec![
            source("xkb", "us"),
            source("xkb", "de"),
            source("ibus", "wayexpand"),
        ];
        let (restored, active) = restore_plan(
            &original_sources,
            &original_active,
            &current_sources,
            &source("ibus", "wayexpand"),
        )
        .unwrap();
        assert_eq!(restored, original_sources);
        assert_eq!(active, original_active);
    }

    #[test]
    fn rollback_preserves_an_unrelated_source_change_made_by_the_user() {
        let original_sources = vec![source("xkb", "us"), source("xkb", "de")];
        let original_active = source("xkb", "us");
        let mut current_sources = original_sources.clone();
        current_sources.extend([source("ibus", "wayexpand"), source("xkb", "fr")]);
        let (restored, active) = restore_plan(
            &original_sources,
            &original_active,
            &current_sources,
            &source("xkb", "fr"),
        )
        .unwrap();
        assert_eq!(
            restored,
            [
                source("xkb", "us"),
                source("xkb", "de"),
                source("xkb", "fr")
            ]
        );
        assert_eq!(active, source("xkb", "fr"));
    }

    #[test]
    fn rollback_does_not_remove_a_preexisting_wayexpand_source() {
        let original_sources = vec![source("xkb", "us"), source("ibus", "wayexpand")];
        let original_active = source("ibus", "wayexpand");
        let (restored, active) = restore_plan(
            &original_sources,
            &original_active,
            &original_sources,
            &original_active,
        )
        .unwrap();
        assert_eq!(restored, original_sources);
        assert_eq!(active, original_active);
    }
}
