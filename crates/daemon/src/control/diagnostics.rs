use super::*;

pub(super) fn explain_request(
    line: &str,
    queue: &std::sync::mpsc::SyncSender<ExplainRequest>,
) -> Result<Option<String>> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (text, json) = if let Some(text) = line.strip_prefix("explain-json ") {
        (text, true)
    } else if let Some(text) = line.strip_prefix("explain ") {
        (text, false)
    } else {
        return Ok(None);
    };
    if !(1..=256).contains(&text.chars().count()) || text.chars().any(char::is_control) {
        return Ok(Some("invalid text\n".into()));
    }
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    let request = ExplainRequest {
        text: text.to_owned(),
        json,
        reply,
    };
    if queue.try_send(request).is_err() {
        return Ok(Some(if json {
            "{\"error\":\"busy\",\"retryable\":true}\n".into()
        } else {
            "explain unavailable: daemon is busy; retry\n".into()
        }));
    }
    Ok(Some(
        answer
            .recv_timeout(EXPLAIN_REPLY_TIMEOUT)
            .unwrap_or_else(|_| "explain unavailable: the daemon did not answer in time\n".into()),
    ))
}

pub(super) fn explain_worker(
    requests: std::sync::mpsc::Receiver<ExplainRequest>,
    snapshot: Arc<Mutex<Option<ExplainSnapshot>>>,
    detailed: bool,
    pause_requested: Arc<AtomicBool>,
) {
    let mut engine: Option<wayexpand_core::ExpansionEngine> = None;
    let mut validated: Option<Arc<wayexpand_core::ValidatedConfig>> = None;
    while let Ok(request) = requests.recv() {
        let started = Instant::now();
        let current = snapshot.lock().ok().and_then(|snapshot| snapshot.clone());
        let answer = current.map_or_else(
            || "explain unavailable: diagnostics are starting; retry\n".to_owned(),
            |state| {
                if validated
                    .as_ref()
                    .is_none_or(|previous| !Arc::ptr_eq(previous, &state.validated))
                {
                    engine = Some(wayexpand_core::ExpansionEngine::from_validated(Arc::clone(
                        &state.validated,
                    )));
                    validated = Some(Arc::clone(&state.validated));
                }
                let Some(engine) = engine.as_mut() else {
                    return "explain unavailable: diagnostics could not initialize\n".to_owned();
                };
                // `pause` and `resume` reply before the reactor applies
                // them; answer from the request rather than a stale snapshot.
                let requested_pause = pause_requested.load(Ordering::Acquire);
                engine.set_user_paused(if requested_pause == state.requested_pause {
                    state.paused
                } else {
                    requested_pause || state.profile_switch_pending
                });
                engine.set_sensitive_focus(state.sensitive);
                engine.set_composition_active(state.composition);
                engine.set_form_open_for_diagnostics(state.form_open);
                engine.set_current_window(state.window.as_deref().cloned());
                engine.set_form_replacement_guarantee(state.form_guarantee);
                engine.set_commands_disabled(state.commands_disabled);
                engine.set_direct_commands_disabled(state.direct_commands_disabled);
                engine.set_title_matching_disabled(state.title_matching_disabled);

                let mut explanation = engine.explain_diagnostic(&request.text, state.backend);
                explanation.push(
                    "daemon",
                    wayexpand_core::CheckStatus::Pass,
                    format!(
                        "running with {} capture and {} output",
                        state.capture, state.output
                    ),
                );
                if state.output == "none" {
                    explanation.push(
                        "output backend",
                        wayexpand_core::CheckStatus::Info,
                        "no output backend (test mode): matches are reported, not typed",
                    );
                } else if state.output_connected {
                    explanation.push(
                        "output backend",
                        wayexpand_core::CheckStatus::Pass,
                        "connected",
                    );
                    if explanation.snippet.is_some() {
                        let route_detail = state.capabilities.as_ref().map_or_else(
                            || "route-specific replacement compatibility is not evaluated by control-socket diagnostics".to_owned(),
                            |capabilities| format!(
                                "{} selected; replacement compatibility is not evaluated by control-socket diagnostics",
                                capabilities.insertion_mode
                            ),
                        );
                        explanation.push(
                            "output route",
                            wayexpand_core::CheckStatus::Info,
                            route_detail,
                        );
                    }
                } else {
                    explanation.push(
                        "output backend",
                        wayexpand_core::CheckStatus::Fail,
                        format!("not connected ({})", state.connection_detail),
                    );
                }
                if started.elapsed() > EXPLAIN_REPLY_TIMEOUT {
                    return if request.json {
                        "{\"error\":\"deadline\",\"retryable\":true}\n".to_owned()
                    } else {
                        "explain unavailable: diagnostics exceeded their time budget; retry\n"
                            .to_owned()
                    };
                }
                if detailed {
                    detailed_explain_response(&explanation, request.json)
                } else {
                    restricted_explain_response(&explanation, request.json)
                }
            },
        );
        let _ = request.reply.try_send(answer);
    }
}

/// Summary fields shared by the detailed and restricted responses.
fn explain_summary(explanation: &wayexpand_core::Explanation) -> (bool, bool, &'static str) {
    let matched = explanation.snippet.is_some();
    let unavailable = explanation
        .checks
        .iter()
        .any(|check| check.name == "diagnostics");
    let would_expand = matched && explanation.suppressed_by().is_none();
    let suppression_reason = if unavailable {
        "unavailable"
    } else if !matched {
        "no_match"
    } else if would_expand {
        "none"
    } else {
        "blocked"
    };
    (matched, would_expand, suppression_reason)
}

/// The full explanation for the user who owns the daemon. Anything this
/// reveals (triggers, checks, command paths) is already readable by that user
/// in their own configuration, and replacements are never rendered here.
fn detailed_explain_response(explanation: &wayexpand_core::Explanation, json: bool) -> String {
    let (matched, would_expand, suppression_reason) = explain_summary(explanation);
    if json {
        format!(
            "{}\n",
            serde_json::json!({
                "matched": matched,
                "would_expand": would_expand,
                "suppression_reason": suppression_reason,
                "suppressed_by": explanation.suppressed_by().map(|check| check.name),
                "typed": explanation.typed,
                "snippet": explanation.snippet,
                "checks": explanation.checks,
            })
        )
    } else {
        explanation.render_text()
    }
}

/// When administrator policy makes the control socket read-only, explain
/// answers only whether the text would expand, independent of configured
/// trigger names, checks, command metadata, and template-variable details.
pub(super) fn restricted_explain_response(
    explanation: &wayexpand_core::Explanation,
    json: bool,
) -> String {
    let (matched, would_expand, suppression_reason) = explain_summary(explanation);
    if json {
        format!(
            "{}\n",
            serde_json::json!({
                "matched": matched,
                "would_expand": would_expand,
                "suppression_reason": suppression_reason,
            })
        )
    } else {
        format!(
            "Snippet match: {}\nWould expand: {}\nReason: {}\n",
            if matched { "yes" } else { "no" },
            if would_expand { "yes" } else { "no" },
            suppression_reason,
        )
    }
}
