use crate::*;

const MAX_PACK_REVIEW_OUTPUT: usize = 24 * 1024 * 1024;
const MAX_PACK_DIFF_DISPLAY_CHARS: usize = 200_000;

impl GuiApp {
    pub(crate) fn start_pack_operation(&mut self, operation: PackOperation) {
        if self.pack_operation_receiver.is_some() {
            return;
        }
        let source = expand_user_path(self.pack_candidate_path.trim());
        let (sender, receiver) = mpsc::channel();
        let worker_operation = operation.clone();
        let spawned = thread::Builder::new()
            .name("wayexpand-pack-review".into())
            .spawn(move || {
                let result = match &worker_operation {
                    PackOperation::Review => review_pack(&source),
                    PackOperation::Install {
                        replace,
                        allow_actions,
                        fingerprint,
                    } => install_pack(&source, *replace, *allow_actions, fingerprint),
                    PackOperation::Rollback { id, allow_actions } => {
                        rollback_pack(id, *allow_actions)
                    }
                };
                let _ = sender.send((worker_operation, result));
            });
        match spawned {
            Ok(_) => {
                self.pack_error = None;
                self.pack_operation_receiver = Some(receiver);
            }
            Err(error) => self.pack_error = Some(format!("Could not start pack review: {error}")),
        }
    }

    pub(crate) fn poll_pack_operation(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.pack_operation_receiver.as_ref() else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => Some(completion),
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => Some((
                PackOperation::Review,
                Err("Pack worker stopped unexpectedly".into()),
            )),
        };
        let Some((operation, result)) = completion else {
            return;
        };
        self.pack_operation_receiver = None;
        match (operation, result) {
            (
                PackOperation::Review,
                Ok(PackWorkerResult::Review {
                    details,
                    installed,
                    diff,
                    diff_error,
                }),
            ) => {
                self.pack_review = Some(PackReview {
                    details,
                    installed,
                    diff,
                    diff_error,
                    install_output: None,
                });
                self.pack_action_consent = false;
                self.pack_rollback_action_consent = false;
                self.pack_rollback_confirmation = None;
                self.pack_error = None;
            }
            (
                operation @ (PackOperation::Install { .. } | PackOperation::Rollback { .. }),
                Ok(PackWorkerResult::Installed(output)),
            ) => {
                if let Some(review) = self.pack_review.as_mut() {
                    review.install_output = Some(output);
                }
                self.pack_rollback_confirmation = None;
                self.pack_error = None;
                self.status = Status::success(match operation {
                    PackOperation::Rollback { .. } => self.strings.pack_rollback_complete(),
                    _ => self.strings.pack_install_complete(),
                });
                self.refresh_diagnostics(true);
            }
            (_, Err(error)) => {
                self.pack_error = Some(error.clone());
                self.status = Status::error(error);
            }
            _ => self.pack_error = Some("Pack worker returned an unexpected result".into()),
        }
    }

    pub(crate) fn render_pack_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.pack_dialog_open {
            return;
        }
        let mut open = self.pack_dialog_open;
        egui::Window::new(self.strings.pack_review_title())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([680.0, 620.0])
            .min_size([520.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(self.strings.pack_review_source());
                ui.add_enabled_ui(self.pack_operation_receiver.is_none(), |ui| {
                    let response = ui.add(
                        TextEdit::singleline(&mut self.pack_candidate_path)
                            .hint_text("~/src/team-packs/packs/operations")
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        self.pack_review = None;
                        self.pack_action_consent = false;
                        self.pack_rollback_action_consent = false;
                        self.pack_rollback_confirmation = None;
                        self.pack_error = None;
                    }
                });
                ui.horizontal(|ui| {
                    if theme::secondary_button(ui, palette, self.strings.pack_review_button())
                        .clicked()
                        && self.pack_operation_receiver.is_none()
                    {
                        self.pack_review = None;
                        self.pack_action_consent = false;
                        self.pack_rollback_action_consent = false;
                        self.pack_rollback_confirmation = None;
                        self.start_pack_operation(PackOperation::Review);
                    }
                    if self.pack_operation_receiver.is_some() {
                        ui.spinner();
                        ui.label(self.strings.pack_operation_running());
                    }
                });
                if let Some(error) = self.pack_error.as_deref() {
                    ui.colored_label(palette.danger, error);
                }
                if let Some(review) = self.pack_review.clone() {
                    ui.separator();
                    self.render_pack_review(ui, palette, &review);
                }
            });
        self.pack_dialog_open = open;
    }

    fn render_pack_review(&mut self, ui: &mut egui::Ui, palette: &Palette, review: &PackReview) {
        let details = &review.details;
        let manifest = &details["manifest"];
        ui.horizontal_wrapped(|ui| {
            ui.heading(format!(
                "{} · {}",
                json_string(manifest, "name"),
                json_string(manifest, "version")
            ));
            let signature_color = signature_color(&details["signature"], palette);
            theme::pill(
                ui,
                json_string(&details["signature"], "state"),
                signature_color,
                theme::tint(signature_color, 36),
            );
        });
        ui.label(format!(
            "{} · {} · {}",
            json_string(manifest, "id"),
            json_string(manifest, "publisher"),
            json_string(manifest, "description")
        ));
        if let Some(review_record) = manifest.get("review").filter(|value| value.is_object()) {
            ui.label(self.strings.pack_review_record(
                &json_string(review_record, "reviewed_by"),
                &json_string(review_record, "reviewed_at"),
                &json_string(review_record, "summary"),
            ));
        } else {
            ui.colored_label(palette.warning, self.strings.pack_no_review_record());
        }
        ui.label(
            self.strings
                .pack_capability_disclosure(&json_string_array(&details["required_capabilities"])),
        );
        ui.label(self.strings.pack_action_counts(
            details["commands"].as_u64().unwrap_or_default(),
            details["hotkey_count"].as_u64().unwrap_or_default(),
        ));
        let actions = json_string_array(&manifest["allowed_actions"]);
        if !actions.is_empty() {
            ui.label(self.strings.pack_broker_actions(&actions));
        }
        let declared_capabilities = json_string_array(&manifest["capabilities"]);
        if !declared_capabilities.is_empty() {
            ui.label(
                self.strings
                    .pack_declared_capabilities(&declared_capabilities),
            );
        }
        if let Some(minimum) = manifest["min_wayexpand_version"].as_str() {
            ui.label(self.strings.pack_minimum_version(minimum));
        }
        self.render_pack_policy(ui, palette, details);
        if let Some(diff_error) = review.diff_error.as_deref() {
            ui.colored_label(palette.warning, diff_error);
        }
        if let Some(diff) = review.diff.as_deref() {
            ui.collapsing(self.strings.pack_diff_title(), |ui| {
                render_untrusted_text(ui, diff, MAX_PACK_DIFF_DISPLAY_CHARS);
            });
        }
        self.render_pack_content(ui, palette, details);
        let needs_action_consent = details["install_policy"]["explicit_action_consent_required"]
            .as_bool()
            .unwrap_or(false);
        if needs_action_consent {
            ui.checkbox(
                &mut self.pack_action_consent,
                self.strings.pack_allow_actions_consent(),
            );
        }
        let content_complete = review_content_is_complete(review);
        if !content_complete {
            ui.colored_label(
                palette.warning,
                if review.installed && review.diff.is_none() {
                    self.strings.pack_diff_required()
                } else {
                    self.strings.pack_review_truncated()
                },
            );
        }
        let can_install = content_complete
            && pack_policy_allows(details)
            && (!needs_action_consent || self.pack_action_consent)
            && self.pack_operation_receiver.is_none();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_install,
                    egui::Button::new(if review.installed {
                        self.strings.pack_install_update()
                    } else {
                        self.strings.pack_install_button()
                    }),
                )
                .clicked()
            {
                let allow_actions = needs_action_consent && self.pack_action_consent;
                let fingerprint = details["fingerprint"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.start_pack_operation(PackOperation::Install {
                    replace: review.installed,
                    allow_actions,
                    fingerprint,
                });
            }
            if !can_install {
                ui.label(
                    RichText::new(self.strings.pack_install_blocked())
                        .small()
                        .color(palette.warning),
                );
            }
        });
        if let Some(output) = review.install_output.as_deref() {
            ui.colored_label(palette.success, self.strings.pack_install_result(output));
        }

        if review.installed {
            let pack_id = json_string(manifest, "id");
            let confirming = self.pack_rollback_confirmation.as_deref() == Some(&pack_id);
            ui.checkbox(
                &mut self.pack_rollback_action_consent,
                self.strings.pack_rollback_allow_actions(),
            );
            let button_label = if confirming {
                self.strings.pack_rollback_confirm()
            } else {
                self.strings.pack_rollback_button()
            };
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.pack_operation_receiver.is_none(),
                        egui::Button::new(button_label),
                    )
                    .clicked()
                {
                    if confirming {
                        let allow_actions = self.pack_rollback_action_consent;
                        self.start_pack_operation(PackOperation::Rollback {
                            id: pack_id.clone(),
                            allow_actions,
                        });
                    } else {
                        self.pack_rollback_confirmation = Some(pack_id.clone());
                    }
                }
                ui.label(
                    RichText::new(self.strings.pack_rollback_explanation())
                        .small()
                        .color(palette.muted),
                );
            });
        }
    }

    fn render_pack_policy(
        &self,
        ui: &mut egui::Ui,
        palette: &Palette,
        details: &serde_json::Value,
    ) {
        let policy = &details["install_policy"];
        if policy["signature_required"].as_bool() == Some(true)
            && details["signature"]["state"] != "verified"
        {
            ui.colored_label(palette.danger, self.strings.pack_signature_required());
        }
        if policy["pack_allowed"].as_bool() == Some(false) {
            ui.colored_label(palette.danger, self.strings.pack_denied_by_policy());
        }
        if details["commands"].as_u64().unwrap_or_default() > 0
            && policy["commands_allowed"].as_bool() == Some(false)
        {
            ui.colored_label(palette.danger, self.strings.pack_commands_denied());
        }
        if details["hotkey_count"].as_u64().unwrap_or_default() > 0
            && policy["hotkeys_allowed"].as_bool() == Some(false)
        {
            ui.colored_label(palette.danger, self.strings.pack_hotkeys_denied());
        }
        match details["signature"]["state"].as_str() {
            Some("unsigned") => {
                ui.colored_label(palette.warning, self.strings.pack_unsigned_warning());
            }
            Some("unverified") | Some("untrusted_signer_file") => {
                ui.colored_label(palette.warning, self.strings.pack_signature_unverified());
            }
            _ => {}
        }
    }

    fn render_pack_content(
        &self,
        ui: &mut egui::Ui,
        _palette: &Palette,
        details: &serde_json::Value,
    ) {
        let expansions = details["expansions"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        ui.collapsing(self.strings.pack_snippets(expansions.len()), |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for expansion in expansions {
                        let title = format!(
                            "{} · {}",
                            json_string(expansion, "trigger"),
                            json_string(expansion, "description")
                        );
                        ui.collapsing(title, |ui| {
                            ui.label(
                                self.strings
                                    .pack_source_file(&json_string(expansion, "file")),
                            );
                            render_untrusted_text(
                                ui,
                                expansion["replacement"].as_str().unwrap_or_default(),
                                MAX_REVIEW_TEXT_CHARS,
                            );
                            if let Some(command) =
                                expansion.get("command").filter(|value| value.is_object())
                            {
                                render_command_review(ui, command);
                            }
                        });
                    }
                    if let Some(hotkeys) = details["hotkeys"].as_array() {
                        for hotkey in hotkeys {
                            ui.collapsing(
                                format!(
                                    "{} · {}",
                                    json_string(hotkey, "chord"),
                                    json_string(hotkey, "description")
                                ),
                                |ui| {
                                    ui.label(
                                        self.strings.pack_source_file(&json_string(hotkey, "file")),
                                    );
                                    render_command_review(ui, &hotkey["command"]);
                                },
                            );
                        }
                    }
                });
        });
    }
}

const MAX_REVIEW_TEXT_CHARS: usize = 64_000;

fn review_content_is_complete(review: &PackReview) -> bool {
    if review.installed && review.diff.is_none() {
        return false;
    }
    if review
        .diff
        .as_deref()
        .is_some_and(|diff| diff.contains("review output clipped"))
    {
        return false;
    }
    let details = &review.details;
    for expansion in details["expansions"].as_array().into_iter().flatten() {
        if expansion["replacement"]
            .as_str()
            .is_some_and(|text| text.chars().count() > MAX_REVIEW_TEXT_CHARS)
            || command_review_is_too_large(&expansion["command"])
        {
            return false;
        }
    }
    details["hotkeys"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|hotkey| !command_review_is_too_large(&hotkey["command"]))
}

fn command_review_is_too_large(command: &serde_json::Value) -> bool {
    command["program"]
        .as_str()
        .is_some_and(|value| value.len() > MAX_REVIEW_TEXT_CHARS)
        || command["args"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|value| {
                value
                    .as_str()
                    .is_some_and(|text| text.len() > MAX_REVIEW_TEXT_CHARS)
            })
}

fn pack_policy_allows(details: &serde_json::Value) -> bool {
    let policy = &details["install_policy"];
    let signature_ok = policy["signature_required"].as_bool() != Some(true)
        || details["signature"]["state"] == "verified";
    signature_ok
        && policy["pack_allowed"].as_bool() != Some(false)
        && (details["commands"].as_u64().unwrap_or_default() == 0
            || policy["commands_allowed"].as_bool() != Some(false))
        && (details["hotkey_count"].as_u64().unwrap_or_default() == 0
            || policy["hotkeys_allowed"].as_bool() != Some(false))
        && policy["signer_override_refused"].is_null()
        && details["fingerprint"]
            .as_str()
            .is_some_and(|value| value.len() == 64)
}

fn signature_color(signature: &serde_json::Value, palette: &Palette) -> Color32 {
    match signature["state"].as_str() {
        Some("verified") => palette.success,
        Some("unsigned") | Some("unverified") => palette.warning,
        _ => palette.danger,
    }
}

fn json_string(value: &serde_json::Value, key: &str) -> String {
    value[key]
        .as_str()
        .map(sanitize_display_text)
        .unwrap_or_else(|| "—".into())
}

fn json_string_array(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(sanitize_display_text)
        .collect()
}

fn sanitize_display_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

fn render_untrusted_text(ui: &mut egui::Ui, text: &str, maximum: usize) {
    let mut shown = clip_chars(&sanitize_display_text(text), maximum);
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .show(ui, |ui| {
            ui.add(
                TextEdit::multiline(&mut shown)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });
}

fn render_command_review(ui: &mut egui::Ui, command: &serde_json::Value) {
    let action = json_string(command, "action");
    if action != "—" {
        ui.colored_label(
            Color32::YELLOW,
            format!("Executable action (not run during review): named broker action {action}"),
        );
    } else {
        let program = json_string(command, "program");
        let args = json_string_array(&command["args"]);
        ui.colored_label(
            Color32::YELLOW,
            format!("Executable action (not run during review): program {program}; argv {args:?}"),
        );
    }
    ui.label(format!(
        "timeout: {} ms",
        command["timeout_ms"].as_u64().unwrap_or_default()
    ));
    ui.label(format!(
        "environment: {}",
        command["environment"].as_str().unwrap_or("minimal")
    ));
    let pass_env = json_string_array(&command["pass_env"]);
    if !pass_env.is_empty() {
        ui.label(format!(
            "forwarded environment variable names: {}",
            pass_env.join(", ")
        ));
    }
}

fn review_pack(source: &std::path::Path) -> Result<PackWorkerResult, String> {
    let source = source.to_string_lossy();
    let output = run_wayexpand(&["pack", "inspect", source.as_ref(), "--json"])?;
    let details: serde_json::Value = serde_json::from_slice(&output)
        .map_err(|error| format!("Pack inspection returned invalid JSON: {error}"))?;
    let id = details["manifest"]["id"]
        .as_str()
        .ok_or_else(|| "Pack inspection did not return an ID".to_owned())?;
    let installed_output = run_wayexpand_optional(&["pack", "path", id]);
    let (installed, diff, diff_error) = match installed_output {
        Some(_) => match run_wayexpand(&["pack", "diff", id, source.as_ref()]) {
            Ok(diff) => {
                let text = String::from_utf8_lossy(&diff);
                (
                    true,
                    Some(clip_chars(&text, MAX_PACK_DIFF_DISPLAY_CHARS)),
                    None,
                )
            }
            Err(error) => (true, None, Some(error)),
        },
        None => (false, None, None),
    };
    Ok(PackWorkerResult::Review {
        details,
        installed,
        diff,
        diff_error,
    })
}

fn install_pack(
    source: &std::path::Path,
    replace: bool,
    allow_actions: bool,
    fingerprint: &str,
) -> Result<PackWorkerResult, String> {
    let source = source.to_string_lossy();
    let mut args = vec!["pack", "install", source.as_ref()];
    if replace {
        args.push("--replace");
    }
    if allow_actions {
        args.push("--allow-actions");
    }
    args.push("--expected-fingerprint");
    args.push(fingerprint);
    let output = run_wayexpand(&args)?;
    Ok(PackWorkerResult::Installed(
        String::from_utf8_lossy(&output).trim().to_owned(),
    ))
}

fn rollback_pack(id: &str, allow_actions: bool) -> Result<PackWorkerResult, String> {
    let mut args = vec!["pack", "rollback", id];
    if allow_actions {
        args.push("--allow-actions");
    }
    let output = run_wayexpand(&args)?;
    Ok(PackWorkerResult::Installed(
        String::from_utf8_lossy(&output).trim().to_owned(),
    ))
}

fn run_wayexpand(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new(super::profile::wayexpand_cli())
        .args(args)
        .output()
        .map_err(|error| format!("Could not start WayExpand CLI: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            format!("WayExpand CLI exited with {}", output.status)
        } else {
            error
        });
    }
    if output.stdout.len() > MAX_PACK_REVIEW_OUTPUT {
        return Err(
            "Pack review output exceeds the GUI limit; use pack inspect in a terminal".into(),
        );
    }
    Ok(output.stdout)
}

fn run_wayexpand_optional(args: &[&str]) -> Option<Vec<u8>> {
    let output = std::process::Command::new(super::profile::wayexpand_cli())
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn clip_chars(value: &str, maximum: usize) -> String {
    let mut output = value.chars().take(maximum).collect::<String>();
    if value.chars().count() > maximum {
        output.push_str("\n… review output clipped …");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{pack_policy_allows, review_content_is_complete};
    use crate::PackReview;

    fn review() -> PackReview {
        PackReview {
            details: serde_json::json!({
                "fingerprint": "a".repeat(64),
                "commands": 0,
                "hotkey_count": 0,
                "signature": { "state": "unsigned" },
                "install_policy": {
                    "signature_required": false,
                    "pack_allowed": true,
                    "commands_allowed": true,
                    "hotkeys_allowed": true,
                    "signer_override_refused": null,
                },
                "expansions": [{
                    "replacement": "reviewable contents",
                    "command": null,
                }],
                "hotkeys": [],
            }),
            installed: false,
            diff: None,
            diff_error: None,
            install_output: None,
        }
    }

    #[test]
    fn pack_review_requires_policy_and_trust_gates_before_enabling_install() {
        let mut review = review();
        assert!(pack_policy_allows(&review.details));

        review.details["install_policy"]["pack_allowed"] = serde_json::Value::Bool(false);
        assert!(!pack_policy_allows(&review.details));
        review.details["install_policy"]["pack_allowed"] = serde_json::Value::Bool(true);

        review.details["install_policy"]["signature_required"] = serde_json::Value::Bool(true);
        assert!(!pack_policy_allows(&review.details));
        review.details["signature"]["state"] = serde_json::Value::String("verified".into());
        assert!(pack_policy_allows(&review.details));

        review.details["install_policy"]["hotkeys_allowed"] = serde_json::Value::Bool(false);
        assert!(pack_policy_allows(&review.details));
        // `hotkeys` is the reviewed hotkey list; the policy gate must use the
        // count that `pack inspect --json` reports separately.
        review.details["hotkey_count"] = serde_json::json!(1);
        review.details["hotkeys"] = serde_json::json!([{ "chord": "Ctrl+Alt+K" }]);
        assert!(!pack_policy_allows(&review.details));
        review.details["install_policy"]["hotkeys_allowed"] = serde_json::Value::Bool(true);
        assert!(pack_policy_allows(&review.details));

        review.details["install_policy"]["signer_override_refused"] =
            serde_json::Value::String("managed trust root".into());
        assert!(!pack_policy_allows(&review.details));
    }

    #[test]
    fn truncated_content_cannot_be_installed_from_the_gui_review() {
        let mut review = review();
        assert!(review_content_is_complete(&review));
        review.installed = true;
        assert!(!review_content_is_complete(&review));
        review.diff = Some("reviewed diff".into());
        assert!(review_content_is_complete(&review));
        review.diff = None;
        review.installed = false;
        review.diff = Some("diff\n… review output clipped …".into());
        assert!(!review_content_is_complete(&review));
        review.diff = None;
        review.details["expansions"][0]["replacement"] =
            serde_json::Value::String("x".repeat(64_001));
        assert!(!review_content_is_complete(&review));
    }
}
