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
                    PackOperation::SearchCatalog { catalog, query } => {
                        search_pack_catalog(catalog, query)
                    }
                    PackOperation::Install {
                        replace,
                        allow_actions,
                        fingerprint,
                    } => install_pack(&source, *replace, *allow_actions, fingerprint),
                    PackOperation::Rollback {
                        id,
                        allow_actions,
                        fingerprint,
                    } => rollback_pack(id, *allow_actions, fingerprint),
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
                PackOperation::SearchCatalog { .. },
                Ok(PackWorkerResult::Catalog {
                    packs,
                    invalid_candidates,
                    signers_file,
                }),
            ) => {
                self.pack_catalog_results = packs;
                self.pack_catalog_invalid_count = invalid_candidates;
                self.pack_catalog_signers_file = Some(signers_file);
                self.pack_error = None;
            }
            (
                PackOperation::Review,
                Ok(PackWorkerResult::Review {
                    details,
                    installed,
                    diff,
                    diff_error,
                    rollback_details,
                    rollback_diff,
                    rollback_diff_error,
                }),
            ) => {
                self.pack_review = Some(PackReview {
                    details,
                    installed,
                    diff,
                    diff_error,
                    rollback_details,
                    rollback_diff,
                    rollback_diff_error,
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
                let rolled_back = matches!(&operation, PackOperation::Rollback { .. });
                if rolled_back {
                    self.pack_review = None;
                } else if let Some(review) = self.pack_review.as_mut() {
                    review.install_output = Some(output);
                }
                self.pack_rollback_confirmation = None;
                self.pack_error = None;
                self.status = Status::success(match operation {
                    PackOperation::Rollback { .. } => self.strings.pack_rollback_complete(),
                    _ => self.strings.pack_install_complete(),
                });
                self.refresh_diagnostics(true);
                if rolled_back {
                    self.start_pack_operation(PackOperation::Review);
                }
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
                self.render_pack_catalog(ui, palette);
                ui.separator();
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

    fn render_pack_catalog(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        theme::section_header(ui, "", self.strings.pack_catalog_title());
        let catalog_changed = ui
            .add_enabled_ui(self.pack_operation_receiver.is_none(), |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.pack_catalog_path)
                        .hint_text("~/src/team-packs")
                        .desired_width(f32::INFINITY),
                )
                .changed()
            })
            .inner;
        if catalog_changed {
            self.pack_catalog_results.clear();
            self.pack_catalog_invalid_count = 0;
            self.pack_catalog_signers_file = None;
        }
        ui.horizontal(|ui| {
            ui.add_enabled_ui(self.pack_operation_receiver.is_none(), |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.pack_catalog_query)
                        .hint_text(self.strings.pack_catalog_query_hint())
                        .desired_width(180.0),
                );
            });
            if ui
                .add_enabled(
                    self.pack_operation_receiver.is_none()
                        && !self.pack_catalog_path.trim().is_empty(),
                    egui::Button::new(self.strings.pack_catalog_search()),
                )
                .clicked()
            {
                self.pack_review = None;
                self.pack_error = None;
                self.pack_catalog_results.clear();
                self.pack_catalog_signers_file = None;
                self.start_pack_operation(PackOperation::SearchCatalog {
                    catalog: expand_user_path(self.pack_catalog_path.trim()),
                    query: self.pack_catalog_query.trim().to_owned(),
                });
            }
            if self.pack_operation_receiver.is_some() {
                ui.spinner();
            }
        });
        if self.pack_catalog_invalid_count > 0 {
            ui.colored_label(
                palette.warning,
                self.strings
                    .pack_catalog_invalid_count(self.pack_catalog_invalid_count),
            );
        }
        if let Some(signers_file) = self.pack_catalog_signers_file.as_deref() {
            ui.label(
                self.strings
                    .pack_catalog_signer_file(&sanitize_display_text(signers_file)),
            );
        }
        let packs = self.pack_catalog_results.clone();
        let catalog = expand_user_path(self.pack_catalog_path.trim());
        for pack in packs {
            let title = format!(
                "{} · {} · {}",
                json_string(&pack, "name"),
                json_string(&pack, "version"),
                json_string(&pack["signature"], "state")
            );
            ui.collapsing(title, |ui| {
                ui.label(format!(
                    "{} · {} · {}",
                    json_string(&pack, "id"),
                    json_string(&pack, "publisher"),
                    json_string(&pack, "description")
                ));
                let signature = &pack["signature"];
                if signature["state"] == "verified" {
                    ui.label(
                        self.strings
                            .pack_catalog_verified_signer(&json_string(signature, "signer")),
                    );
                } else if signature["state"] == "unverified" {
                    ui.colored_label(
                        palette.warning,
                        self.strings
                            .pack_catalog_signature_reason(&json_string(signature, "reason")),
                    );
                } else {
                    ui.colored_label(palette.warning, self.strings.pack_unsigned_warning());
                }
                ui.label(self.strings.pack_capability_disclosure(&json_string_array(
                    &pack["required_capabilities"],
                )));
                ui.label(self.strings.pack_action_counts(
                    pack["commands"].as_u64().unwrap_or_default(),
                    pack["hotkeys"].as_u64().unwrap_or_default(),
                ));
                let allowed_actions = json_string_array(&pack["allowed_actions"]);
                if !allowed_actions.is_empty() {
                    ui.label(self.strings.pack_broker_actions(&allowed_actions));
                }
                if let Some(record) = pack.get("review").filter(|value| value.is_object()) {
                    ui.label(self.strings.pack_review_record(
                        &json_string(record, "reviewed_by"),
                        &json_string(record, "reviewed_at"),
                        &json_string(record, "summary"),
                    ));
                } else {
                    ui.colored_label(palette.warning, self.strings.pack_no_review_record());
                }
                if ui
                    .add_enabled(
                        self.pack_operation_receiver.is_none(),
                        egui::Button::new(self.strings.pack_catalog_review_candidate()),
                    )
                    .clicked()
                {
                    match resolve_catalog_candidate(
                        &catalog,
                        pack["path"].as_str().unwrap_or_default(),
                    ) {
                        Ok(path) => {
                            self.pack_candidate_path = path.display().to_string();
                            self.pack_review = None;
                            self.pack_action_consent = false;
                            self.pack_rollback_action_consent = false;
                            self.pack_rollback_confirmation = None;
                            self.start_pack_operation(PackOperation::Review);
                        }
                        Err(error) => self.pack_error = Some(error),
                    }
                }
            });
        }
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
        self.render_signature_details(ui, details);
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
            self.render_rollback_review(ui, palette, review);
        }
    }

    fn render_rollback_review(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        review: &PackReview,
    ) {
        ui.separator();
        theme::section_header(ui, "", self.strings.pack_rollback_review_title());
        let Some(details) = review.rollback_details.as_ref() else {
            if let Some(error) = review.rollback_diff_error.as_deref() {
                ui.colored_label(palette.warning, error);
            } else {
                ui.colored_label(palette.warning, self.strings.pack_rollback_unavailable());
            }
            return;
        };
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
        self.render_signature_details(ui, details);
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
        self.render_pack_policy(ui, palette, details);
        self.render_pack_content(ui, palette, details);
        if let Some(error) = review.rollback_diff_error.as_deref() {
            ui.colored_label(palette.warning, error);
        }
        if let Some(diff) = review.rollback_diff.as_deref() {
            ui.collapsing(self.strings.pack_rollback_diff_title(), |ui| {
                render_untrusted_text(ui, diff, MAX_PACK_DIFF_DISPLAY_CHARS);
            });
        }
        let has_actions = details["commands"].as_u64().unwrap_or_default() > 0
            || details["hotkey_count"].as_u64().unwrap_or_default() > 0;
        if has_actions {
            ui.checkbox(
                &mut self.pack_rollback_action_consent,
                self.strings.pack_rollback_allow_actions(),
            );
        }
        let fingerprint = details["fingerprint"].as_str().unwrap_or_default();
        let confirming = !fingerprint.is_empty()
            && self.pack_rollback_confirmation.as_deref() == Some(fingerprint);
        let review_ready = review.rollback_diff.as_deref().is_some()
            && rollback_review_is_complete(details, review.rollback_diff.as_deref())
            && pack_policy_allows(details)
            && (!has_actions || self.pack_rollback_action_consent);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    review_ready && self.pack_operation_receiver.is_none(),
                    egui::Button::new(if confirming {
                        self.strings.pack_rollback_confirm()
                    } else {
                        self.strings.pack_rollback_button()
                    }),
                )
                .clicked()
            {
                if confirming {
                    let id = json_string(manifest, "id");
                    self.start_pack_operation(PackOperation::Rollback {
                        id,
                        allow_actions: has_actions && self.pack_rollback_action_consent,
                        fingerprint: fingerprint.to_owned(),
                    });
                } else {
                    self.pack_rollback_confirmation = Some(fingerprint.to_owned());
                }
            }
            ui.label(
                RichText::new(self.strings.pack_rollback_explanation())
                    .small()
                    .color(palette.muted),
            );
        });
        if !review_ready {
            ui.colored_label(
                palette.warning,
                self.strings.pack_rollback_review_required(),
            );
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

    fn render_signature_details(&self, ui: &mut egui::Ui, details: &serde_json::Value) {
        let signature = &details["signature"];
        match signature["state"].as_str() {
            Some("verified") => ui.label(self.strings.pack_signature_identity(
                &json_string(signature, "signer"),
                &json_string(signature, "signer_file_source"),
                &json_string(signature, "signer_file"),
            )),
            Some("unverified" | "untrusted_signer_file" | "blocked_by_policy") => ui.label(
                self.strings
                    .pack_signature_reason(&json_string(signature, "reason")),
            ),
            _ => ui.label(self.strings.pack_signature_unsigned_detail()),
        };
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
    pack_content_is_complete(&review.details, review.diff.as_deref(), review.installed)
}

fn rollback_review_is_complete(details: &serde_json::Value, diff: Option<&str>) -> bool {
    details["fingerprint"]
        .as_str()
        .is_some_and(valid_fingerprint)
        && pack_content_is_complete(details, diff, true)
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn pack_content_is_complete(
    details: &serde_json::Value,
    diff: Option<&str>,
    diff_required: bool,
) -> bool {
    let Some(expansions) = details["expansions"].as_array() else {
        return false;
    };
    let Some(hotkeys) = details["hotkeys"].as_array() else {
        return false;
    };
    let Some(expansion_count) = details["expansion_count"].as_u64() else {
        return false;
    };
    let Some(hotkey_count) = details["hotkey_count"].as_u64() else {
        return false;
    };
    let Some(command_count) = details["commands"].as_u64() else {
        return false;
    };
    let manifest_fields = ["id", "version", "name", "publisher", "description"];
    let Some(capabilities) = details["required_capabilities"].as_array() else {
        return false;
    };
    if !signature_review_is_complete(&details["signature"]) {
        return false;
    }
    if manifest_fields.iter().any(|key| {
        details["manifest"]
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_none_or(|text| text.chars().count() > MAX_REVIEW_TEXT_CHARS)
    }) || capabilities.iter().any(|capability| {
        capability
            .as_str()
            .is_none_or(|text| text.chars().count() > MAX_REVIEW_TEXT_CHARS)
    }) || expansion_count != expansions.len() as u64
        || hotkey_count != hotkeys.len() as u64
    {
        return false;
    }
    if diff_required && diff.is_none() {
        return false;
    }
    if diff.is_some_and(|diff| diff.contains("review output clipped")) {
        return false;
    }
    let mut reviewed_commands = 0u64;
    for expansion in expansions {
        if ["file", "id", "trigger", "description", "replacement"]
            .iter()
            .any(|key| {
                expansion
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|text| text.chars().count() > MAX_REVIEW_TEXT_CHARS)
            })
        {
            return false;
        }
        match expansion.get("command") {
            Some(serde_json::Value::Null) => {}
            Some(command) if command_review_is_complete(command) => reviewed_commands += 1,
            _ => return false,
        }
    }
    for hotkey in hotkeys {
        if ["file", "chord", "description"].iter().any(|key| {
            hotkey
                .get(key)
                .and_then(serde_json::Value::as_str)
                .is_none_or(|text| text.chars().count() > MAX_REVIEW_TEXT_CHARS)
        }) || !hotkey
            .get("command")
            .is_some_and(command_review_is_complete)
        {
            return false;
        }
        reviewed_commands += 1;
    }
    reviewed_commands == command_count
}

fn signature_review_is_complete(signature: &serde_json::Value) -> bool {
    let Some(state) = signature["state"].as_str() else {
        return false;
    };
    match state {
        "unsigned" => true,
        "verified" => {
            ["signer", "signer_file"].iter().all(|key| {
                signature[*key].as_str().is_some_and(|value| {
                    !value.is_empty() && value.chars().count() <= MAX_REVIEW_TEXT_CHARS
                })
            }) && matches!(
                signature["signer_file_source"].as_str(),
                Some("configured" | "explicit")
            )
        }
        "unverified" | "untrusted_signer_file" | "blocked_by_policy" => {
            signature["reason"].as_str().is_some_and(|reason| {
                !reason.is_empty() && reason.chars().count() <= MAX_REVIEW_TEXT_CHARS
            })
        }
        _ => false,
    }
}

fn command_review_is_complete(command: &serde_json::Value) -> bool {
    let Some(object) = command.as_object() else {
        return false;
    };
    let Some(args) = command["args"].as_array() else {
        return false;
    };
    let Some(pass_env) = command["pass_env"].as_array() else {
        return false;
    };
    let action_valid = command["action"].is_null() || command["action"].is_string();
    let action_is_named = command["action"]
        .as_str()
        .is_some_and(|action| !action.is_empty());
    let program_valid = command["program"]
        .as_str()
        .is_some_and(|program| !program.is_empty() || action_is_named);
    object.contains_key("action")
        && action_valid
        && program_valid
        && command["timeout_ms"].as_u64().is_some()
        && matches!(command["environment"].as_str(), Some("minimal" | "inherit"))
        && args.iter().all(|value| {
            value
                .as_str()
                .is_some_and(|text| text.chars().count() <= MAX_REVIEW_TEXT_CHARS)
        })
        && pass_env.iter().all(|value| {
            value
                .as_str()
                .is_some_and(|text| text.chars().count() <= MAX_REVIEW_TEXT_CHARS)
        })
        && ["program", "action"]
            .iter()
            .filter_map(|key| command[*key].as_str())
            .all(|value| value.chars().count() <= MAX_REVIEW_TEXT_CHARS)
}

fn pack_policy_allows(details: &serde_json::Value) -> bool {
    let policy = &details["install_policy"];
    let Some(signature_required) = policy["signature_required"].as_bool() else {
        return false;
    };
    let Some(pack_allowed) = policy["pack_allowed"].as_bool() else {
        return false;
    };
    let Some(commands_allowed) = policy["commands_allowed"].as_bool() else {
        return false;
    };
    let Some(hotkeys_allowed) = policy["hotkeys_allowed"].as_bool() else {
        return false;
    };
    let Some(action_consent_required) = policy["explicit_action_consent_required"].as_bool() else {
        return false;
    };
    if policy["safe_mode"].as_bool().is_none()
        || policy.get("signer_override_refused") != Some(&serde_json::Value::Null)
        || !valid_fingerprint(details["fingerprint"].as_str().unwrap_or_default())
    {
        return false;
    }
    let Some(command_count) = details["commands"].as_u64() else {
        return false;
    };
    let Some(hotkey_count) = details["hotkey_count"].as_u64() else {
        return false;
    };
    let Some(signature_state) = details["signature"]["state"].as_str() else {
        return false;
    };
    let signature_ok = matches!(signature_state, "verified" | "unsigned" | "unverified")
        && (!signature_required || signature_state == "verified");
    signature_ok
        && pack_allowed
        && (command_count == 0 || commands_allowed)
        && (hotkey_count == 0 || hotkeys_allowed)
        && action_consent_required == (command_count > 0 || hotkey_count > 0)
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

fn search_pack_catalog(catalog: &std::path::Path, query: &str) -> Result<PackWorkerResult, String> {
    let output = std::process::Command::new(super::profile::wayexpand_cli())
        .arg("pack")
        .arg("search")
        .arg(catalog)
        .arg(query)
        .arg("--json")
        .output()
        .map_err(|error| format!("Could not start WayExpand catalog search: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            format!("WayExpand CLI exited with {}", output.status)
        } else {
            error
        });
    }
    if output.stdout.len() > MAX_PACK_REVIEW_OUTPUT {
        return Err("Catalog search output exceeds the GUI limit".into());
    }
    let response: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Catalog search returned invalid JSON: {error}"))?;
    let Some(packs) = response["packs"].as_array() else {
        return Err("Catalog search response did not contain a pack list".into());
    };
    let Some(invalid_candidates) = response["invalid_candidates"].as_u64() else {
        return Err("Catalog search response did not report invalid entries".into());
    };
    if packs
        .iter()
        .any(|pack| !catalog_candidate_is_complete(pack))
    {
        return Err("Catalog search returned an incomplete pack entry".into());
    }
    let Some(signers_file) = response["signers_file"].as_str() else {
        return Err("Catalog search response did not identify its signer list".into());
    };
    Ok(PackWorkerResult::Catalog {
        packs: packs.clone(),
        invalid_candidates: usize::try_from(invalid_candidates).unwrap_or(usize::MAX),
        signers_file: signers_file.to_owned(),
    })
}

fn catalog_candidate_is_complete(pack: &serde_json::Value) -> bool {
    ["id", "version", "name", "publisher", "description", "path"]
        .iter()
        .all(|key| pack.get(key).and_then(serde_json::Value::as_str).is_some())
        && pack["required_capabilities"]
            .as_array()
            .is_some_and(|values| values.iter().all(serde_json::Value::is_string))
        && pack["allowed_actions"]
            .as_array()
            .is_some_and(|values| values.iter().all(serde_json::Value::is_string))
        && pack["commands"].as_u64().is_some()
        && pack["hotkeys"].as_u64().is_some()
        && match pack["signature"]["state"].as_str() {
            Some("verified") => pack["signature"]["signer"].as_str().is_some(),
            Some("unsigned") => true,
            Some("unverified") => pack["signature"]["reason"].as_str().is_some(),
            _ => false,
        }
}

fn resolve_catalog_candidate(
    catalog: &std::path::Path,
    relative_pack_path: &str,
) -> Result<std::path::PathBuf, String> {
    use std::path::Component;

    let relative = std::path::Path::new(relative_pack_path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Catalog returned a pack path outside its root".into());
    }
    let metadata = std::fs::symlink_metadata(catalog)
        .map_err(|error| format!("Could not inspect pack catalog: {error}"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("Pack catalog must be a real directory".into());
    }
    let root = std::fs::canonicalize(catalog)
        .map_err(|error| format!("Could not resolve pack catalog: {error}"))?;
    let nested_pack_root = catalog.join("packs");
    let use_nested_pack_root = std::fs::symlink_metadata(&nested_pack_root).is_ok_and(|metadata| {
        metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !catalog.join("wayexpand-pack.toml").exists()
    });
    let selected_root = if use_nested_pack_root {
        let resolved = std::fs::canonicalize(&nested_pack_root)
            .map_err(|error| format!("Could not resolve catalog packs directory: {error}"))?;
        if !resolved.starts_with(&root) {
            return Err("Catalog packs directory resolves outside its root".into());
        }
        nested_pack_root
    } else {
        catalog.to_path_buf()
    };
    let candidate = selected_root.join(relative);
    let candidate_metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|error| format!("Could not inspect catalog pack: {error}"))?;
    if !candidate_metadata.file_type().is_dir() || candidate_metadata.file_type().is_symlink() {
        return Err("Catalog pack must be a real directory".into());
    }
    let candidate = std::fs::canonicalize(candidate)
        .map_err(|error| format!("Could not resolve catalog pack: {error}"))?;
    let selected_root = std::fs::canonicalize(selected_root)
        .map_err(|error| format!("Could not resolve catalog pack root: {error}"))?;
    if !candidate.starts_with(selected_root) {
        return Err("Catalog pack resolves outside its root".into());
    }
    Ok(candidate)
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
    let (rollback_details, rollback_diff, rollback_diff_error) = if installed {
        match run_wayexpand(&["pack", "rollback-review", id, "--json"]) {
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(details) => {
                    let backup_path = details["backup_path"].as_str();
                    let rollback_diff = backup_path
                        .map(|backup| run_wayexpand(&["pack", "diff", id, backup]))
                        .transpose();
                    match rollback_diff {
                        Ok(Some(diff)) => (
                            Some(details),
                            Some(clip_chars(
                                &String::from_utf8_lossy(&diff),
                                MAX_PACK_DIFF_DISPLAY_CHARS,
                            )),
                            None,
                        ),
                        Ok(None) => (
                            Some(details),
                            None,
                            Some("Rollback review did not return a backup path".into()),
                        ),
                        Err(error) => (Some(details), None, Some(error)),
                    }
                }
                Err(error) => (
                    None,
                    None,
                    Some(format!("Rollback review returned invalid JSON: {error}")),
                ),
            },
            Err(error) => (None, None, Some(error)),
        }
    } else {
        (None, None, None)
    };
    Ok(PackWorkerResult::Review {
        details,
        installed,
        diff,
        diff_error,
        rollback_details,
        rollback_diff,
        rollback_diff_error,
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

fn rollback_pack(
    id: &str,
    allow_actions: bool,
    fingerprint: &str,
) -> Result<PackWorkerResult, String> {
    let mut args = vec!["pack", "rollback", id];
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
    use super::{
        catalog_candidate_is_complete, pack_policy_allows, resolve_catalog_candidate,
        review_content_is_complete, rollback_review_is_complete,
    };
    use crate::PackReview;

    fn review() -> PackReview {
        PackReview {
            details: serde_json::json!({
                "manifest": {
                    "id": "example",
                    "version": "1.0.0",
                    "name": "Example",
                    "publisher": "WayExpand",
                    "description": "Reviewed fixture"
                },
                "fingerprint": "a".repeat(64),
                "expansion_count": 1,
                "required_capabilities": [],
                "commands": 0,
                "hotkey_count": 0,
                "signature": { "state": "unsigned" },
                "install_policy": {
                    "safe_mode": false,
                    "signature_required": false,
                    "pack_allowed": true,
                    "commands_allowed": true,
                    "hotkeys_allowed": true,
                    "explicit_action_consent_required": false,
                    "signer_override_refused": null,
                },
                "expansions": [{
                    "file": "snippets.toml",
                    "id": "example",
                    "trigger": ":example",
                    "description": "Example",
                    "replacement": "reviewable contents",
                    "command": null,
                }],
                "hotkeys": [],
            }),
            installed: false,
            diff: None,
            diff_error: None,
            rollback_details: None,
            rollback_diff: None,
            rollback_diff_error: None,
            install_output: None,
        }
    }

    #[test]
    fn pack_review_requires_policy_and_trust_gates_before_enabling_install() {
        let mut review = review();
        assert!(review_content_is_complete(&review));
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
        review.details["commands"] = serde_json::json!(1);
        review.details["install_policy"]["explicit_action_consent_required"] =
            serde_json::Value::Bool(true);
        review.details["hotkeys"] = serde_json::json!([{
            "file": "hotkeys.toml",
            "chord": "Ctrl+Alt+K",
            "description": "Action",
            "command": {
                "action": "restart-service",
                "program": "",
                "args": [],
                "timeout_ms": 1000,
                "environment": "minimal",
                "pass_env": []
            }
        }]);
        assert!(!pack_policy_allows(&review.details));
        review.details["install_policy"]["hotkeys_allowed"] = serde_json::Value::Bool(true);
        assert!(pack_policy_allows(&review.details));

        review.details["install_policy"]["signer_override_refused"] =
            serde_json::Value::String("managed trust root".into());
        assert!(!pack_policy_allows(&review.details));
    }

    #[test]
    fn incomplete_or_inconsistent_pack_review_fails_closed() {
        let mut candidate = review();
        candidate
            .details
            .as_object_mut()
            .unwrap()
            .remove("expansions");
        assert!(!review_content_is_complete(&candidate));

        let mut candidate = review();
        candidate.details["expansion_count"] = serde_json::json!(2);
        assert!(!review_content_is_complete(&candidate));

        let mut candidate = review();
        candidate.details["expansions"][0]
            .as_object_mut()
            .unwrap()
            .remove("trigger");
        assert!(!review_content_is_complete(&candidate));

        let mut candidate = review();
        candidate.details["expansions"][0]["command"] =
            serde_json::json!({ "program": "/usr/bin/example" });
        assert!(!review_content_is_complete(&candidate));

        let mut candidate = review();
        candidate.details["install_policy"]
            .as_object_mut()
            .unwrap()
            .remove("pack_allowed");
        assert!(!pack_policy_allows(&candidate.details));

        let mut candidate = review();
        candidate.details["install_policy"]
            .as_object_mut()
            .unwrap()
            .remove("signer_override_refused");
        assert!(!pack_policy_allows(&candidate.details));

        let mut candidate = review();
        candidate.details["signature"]["state"] = serde_json::json!("blocked_by_policy");
        assert!(!pack_policy_allows(&candidate.details));
    }

    #[test]
    fn verified_signature_review_requires_signer_and_trust_source() {
        let mut candidate = review();
        candidate.details["signature"] = serde_json::json!({ "state": "verified" });
        assert!(!review_content_is_complete(&candidate));

        candidate.details["signature"] = serde_json::json!({
            "state": "verified",
            "signer": "release@example.org",
            "signer_file": "/etc/wayexpand/allowed_signers",
            "signer_file_source": "configured"
        });
        assert!(review_content_is_complete(&candidate));

        candidate.details["signature"]["signer_file_source"] = serde_json::json!("unknown");
        assert!(!review_content_is_complete(&candidate));
    }

    #[test]
    fn catalog_search_requires_disclosure_fields_before_display() {
        let candidate = serde_json::json!({
            "id": "ops-linux",
            "version": "1.2.0",
            "name": "Linux Operations",
            "publisher": "Example Team",
            "description": "Reviewed commands and replies",
            "required_capabilities": ["forms"],
            "allowed_actions": [],
            "commands": 0,
            "hotkeys": 0,
            "signature": { "state": "verified", "signer": "packs@example.org" },
            "path": "ops-linux"
        });
        assert!(catalog_candidate_is_complete(&candidate));
        let mut missing = candidate;
        missing.as_object_mut().unwrap().remove("signature");
        assert!(!catalog_candidate_is_complete(&missing));
    }

    #[test]
    fn catalog_candidate_resolution_stays_inside_the_local_catalog() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-gui-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pack_root = root.join("packs");
        let pack = pack_root.join("ops-linux");
        std::fs::create_dir_all(&pack).unwrap();
        let resolved = resolve_catalog_candidate(&root, "ops-linux").unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&pack).unwrap());
        assert!(resolve_catalog_candidate(&root, "../outside").is_err());
        std::fs::remove_dir_all(root).unwrap();
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

    #[test]
    fn rollback_requires_a_complete_fingerprinted_backup_and_diff() {
        let details = serde_json::json!({
            "manifest": {
                "id": "example",
                "version": "1.0.0",
                "name": "Example",
                "publisher": "WayExpand",
                "description": "Reviewed fixture"
            },
            "fingerprint": "a".repeat(64),
            "expansion_count": 1,
            "required_capabilities": [],
            "commands": 0,
            "hotkey_count": 0,
            "signature": { "state": "unsigned" },
            "install_policy": {
                "safe_mode": false,
                "signature_required": false,
                "pack_allowed": true,
                "commands_allowed": true,
                "hotkeys_allowed": true,
                "explicit_action_consent_required": false,
                "signer_override_refused": null
            },
            "expansions": [{
                "file": "snippets.toml",
                "id": "example",
                "trigger": ":example",
                "description": "Example",
                "replacement": "reviewed backup",
                "command": null
            }],
            "hotkeys": [],
        });
        assert!(rollback_review_is_complete(&details, Some("complete diff")));
        assert!(!rollback_review_is_complete(&details, None));
        assert!(!rollback_review_is_complete(
            &details,
            Some("diff\n… review output clipped …")
        ));
        let oversized = serde_json::json!({
            "manifest": {
                "id": "example",
                "version": "1.0.0",
                "name": "Example",
                "publisher": "WayExpand",
                "description": "Reviewed fixture"
            },
            "fingerprint": "a".repeat(64),
            "expansion_count": 1,
            "required_capabilities": [],
            "commands": 0,
            "hotkey_count": 0,
            "signature": { "state": "unsigned" },
            "install_policy": {
                "safe_mode": false,
                "signature_required": false,
                "pack_allowed": true,
                "commands_allowed": true,
                "hotkeys_allowed": true,
                "explicit_action_consent_required": false,
                "signer_override_refused": null
            },
            "expansions": [{
                "file": "snippets.toml",
                "id": "example",
                "trigger": ":example",
                "description": "Example",
                "replacement": "x".repeat(64_001),
                "command": null
            }],
            "hotkeys": [],
        });
        assert!(!rollback_review_is_complete(
            &oversized,
            Some("complete diff")
        ));
    }
}
