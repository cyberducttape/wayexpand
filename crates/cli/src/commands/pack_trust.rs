//! The signature, organization-policy and action-consent gate every pack
//! installation passes, whether from `pack install`, `pack update` or
//! `pack sync`.

use super::config::{install_local_pack, pack_signers_override_refusal};
use crate::*;

pub(crate) struct PackTrust {
    pub(crate) policy: wayexpand_core::OrganizationPolicy,
    pub(crate) signers: PathBuf,
    /// The signer list came from policy rather than `--signers`; the policy
    /// default must be root-owned to be trusted.
    pub(crate) policy_signers: bool,
}

impl PackTrust {
    pub(crate) fn load(signers_override: Option<String>) -> Result<Self> {
        let policy = load_policy()
            .map_err(|error| config_error(format!("organization policy is invalid: {error}")))?;
        let policy_signers = signers_override.is_none();
        let signers = signers_override
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&policy.pack_signers_file));
        Ok(Self {
            policy,
            signers,
            policy_signers,
        })
    }

    fn signatures_required(&self) -> bool {
        self.policy.safe_mode && self.policy.require_signed_packs
    }

    /// Verify a signature when present; require one when policy says so.
    pub(crate) fn verify(&self, path: &Path) -> Result<Option<String>> {
        if let Some(refusal) = pack_signers_override_refusal(&self.policy, !self.policy_signers) {
            return Err(config_error(refusal));
        }
        let verified = if self.policy_signers {
            wayexpand_core::trusted_signers_file(&self.signers)
                .and_then(|()| wayexpand_core::verify_pack_signature(path, &self.signers))
        } else {
            wayexpand_core::verify_pack_signature(path, &self.signers)
        };
        match verified {
            Ok(signer) => Ok(Some(signer)),
            Err(wayexpand_core::PackError::Unsigned) if !self.signatures_required() => Ok(None),
            Err(error) if !self.signatures_required() && !self.signers.exists() => {
                eprintln!(
                    "warning: pack signature not verified ({error}); no signers file at {}",
                    self.signers.display()
                );
                Ok(None)
            }
            Err(error) => Err(config_error(format!("pack rejected: {error}"))),
        }
    }

    /// The signature state to show next to a candidate; never an error.
    pub(crate) fn describe_signature(&self, path: &Path) -> serde_json::Value {
        if let Some(reason) = pack_signers_override_refusal(&self.policy, !self.policy_signers) {
            return serde_json::json!({ "state": "blocked_by_policy", "reason": reason });
        }
        match wayexpand_core::verify_pack_signature(path, &self.signers) {
            Ok(signer) => serde_json::json!({ "state": "verified", "signer": signer }),
            Err(wayexpand_core::PackError::Unsigned) => serde_json::json!({ "state": "unsigned" }),
            Err(error) => serde_json::json!({ "state": "unverified", "reason": error.to_string() }),
        }
    }

    /// Organization policy and explicit consent for command or hotkey
    /// actions. Checked before anything is copied.
    pub(crate) fn admit(
        &self,
        inspection: &wayexpand_core::PackInspection,
        allow_actions: bool,
    ) -> Result<()> {
        let policy = &self.policy;
        if !allow_actions && (inspection.command_count > 0 || inspection.hotkey_count > 0) {
            bail!("refusing to install command or hotkey actions by default; review the pack and pass --allow-actions to activate them");
        }
        if policy.safe_mode && !policy.pack_allowed(&inspection.manifest.id) {
            bail!(
                "organization policy does not allow pack {}; ask an administrator to add it to allowed_packs",
                inspection.manifest.id
            );
        }
        if policy.safe_mode && policy.disable_commands && inspection.command_count > 0 {
            bail!(
                "organization policy disables commands; this pack contains command-backed entries"
            );
        }
        if policy.safe_mode && policy.disable_hotkeys && inspection.hotkey_count > 0 {
            bail!("organization policy disables hotkeys; this pack contains hotkeys");
        }
        Ok(())
    }

    /// Verify, admit and install one reviewed pack. Returns the installed
    /// path and the verified signer, if any.
    pub(crate) fn install(
        &self,
        source: &Path,
        inspection: &wayexpand_core::PackInspection,
        replace: bool,
        allow_downgrade: bool,
        allow_actions: bool,
    ) -> Result<(PathBuf, Option<String>)> {
        let signer = self.verify(source)?;
        self.admit(inspection, allow_actions)?;
        let installed = install_local_pack(
            source,
            inspection,
            replace,
            allow_downgrade,
            signer.is_some().then_some(self.signers.as_path()),
        )?;
        Ok((installed, signer))
    }
}
