use super::*;

#[test]
fn clipboard_variable_is_adopted_by_sensitive_storage_without_copying() {
    let original_pointer = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pointer_from_reader = Arc::clone(&original_pointer);
    let context = TemplateContext {
        clipboard: Some(ClipboardReader(Arc::new(move || {
            let value = String::from("synthetic clipboard secret");
            pointer_from_reader.store(
                value.as_ptr() as usize,
                std::sync::atomic::Ordering::Relaxed,
            );
            Some(value)
        }))),
        ..TemplateContext::default()
    };

    let value = evaluate(
        &Variable::Clipboard,
        "clipboard",
        &context,
        &mut HashMap::new(),
    )
    .unwrap();
    let RenderValue::Owned(value) = value else {
        panic!("clipboard values must have zeroizing ownership");
    };
    assert_eq!(value.as_str(), "synthetic clipboard secret");
    assert_eq!(
        value.as_ptr() as usize,
        original_pointer.load(std::sync::atomic::Ordering::Relaxed),
        "the clipboard String allocation should move into protected storage"
    );
    assert_eq!(format!("{value:?}"), "[REDACTED]");
}

#[test]
fn partial_clipboard_render_failure_does_not_return_rendered_plaintext() {
    let template = "prefix {{clipboard}} {{field:needs_input}}";
    let context = TemplateContext {
        clipboard: Some(ClipboardReader(Arc::new(|| {
            Some("synthetic partial-render secret".to_owned())
        }))),
        ..TemplateContext::default()
    };
    let compiled = CompiledTemplate::compile(template).unwrap();
    for error in [
        render_template(template, &context).unwrap_err(),
        compiled.render_sensitive_with_cursor(&context).unwrap_err(),
    ] {
        assert!(!error
            .to_string()
            .contains("synthetic partial-render secret"));
        assert!(!format!("{error:?}").contains("synthetic partial-render secret"));
    }
}

#[test]
fn local_today_reports_an_unrepresentable_timestamp() {
    assert_eq!(local_date_at(u64::MAX), Err(DateError::Unrepresentable));
    assert_ne!(local_date_at(u64::MAX).ok().as_deref(), Some("1970-01-01"));
}

#[test]
fn preview_renderer_stops_at_its_output_budget_for_large_templates() {
    let context = TemplateContext::default();
    assert_eq!(
        render_template_preview(&"x".repeat(1024 * 1024), &context, 512).unwrap(),
        "x".repeat(512)
    );
    assert_eq!(
        render_template_preview(
            &format!("{}{{{{unknown}}}}", "y".repeat(512)),
            &context,
            512
        )
        .unwrap(),
        "y".repeat(512)
    );
    assert_eq!(
        render_template_preview("a🙂z", &context, 2).unwrap(),
        "a",
        "preview truncation must not split a UTF-8 scalar"
    );
    assert_eq!(
        render_template_preview("hello {{username}}", &context, 512).unwrap(),
        format!("hello {}", context.username)
    );
    assert_eq!(
        render_template_preview(
            &format!("first line\n{}{{{{unknown}}}}", "x".repeat(1024 * 1024)),
            &context,
            512
        )
        .unwrap(),
        "first line"
    );
    assert_eq!(
        render_template_preview(
            &format!("{{{{newline}}}}{}", "x".repeat(1024 * 1024)),
            &context,
            512
        )
        .unwrap(),
        ""
    );

    let mut snippets = HashMap::new();
    snippets.insert(
        "signature".into(),
        format!("included line\n{{{{cursor}}}}{}", "x".repeat(1024 * 1024)).into(),
    );
    let context = TemplateContext {
        snippets: Arc::new(snippets),
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template_preview("{{snippet:signature}}", &context, 512).unwrap(),
        "included line"
    );
    assert!(matches!(
        render_template("{{snippet:signature}}", &context),
        Err(TemplateError::CursorInInclude)
    ));
    assert_eq!(
        render_template("first line\nsecond line", &context).unwrap(),
        "first line\nsecond line"
    );
}

#[test]
fn compiled_templates_render_what_the_streaming_renderer_renders() {
    // Includes and previews still go through the streaming renderer; both
    // share one variable parser and evaluator, and must agree.
    let mut snippets = HashMap::new();
    snippets.insert("sig".into(), "-- {{username}} @ {{hostname}}".into());
    snippets.insert("outer".into(), "[{{snippet:sig}}]".into());
    let mut env = BTreeMap::new();
    env.insert("TEAM".to_owned(), "platform".to_owned());
    let context = TemplateContext {
        username: "ada".into(),
        hostname: "workstation".into(),
        unix_timestamp: 86_400 * 365,
        env: Arc::new(env),
        snippets: Arc::new(snippets),
        ..TemplateContext::default()
    };
    for template in [
        "plain text",
        "",
        "{{username}}{{hostname}}{{unix_timestamp}}",
        "a{{tab}}b{{newline}}c",
        "{{date}} {{time}} {{datetime}} {{date+1d}} {{time-2h}}",
        "{{date:%Y/%m/%d}} {{datetime+1w:%H}}",
        "literal {{{{ braces }}}} kept",
        "}}}} alone and {{{{",
        "team={{env:TEAM}}",
        "{{snippet:outer}} after",
        "  {{  username  }}  ",
    ] {
        let compiled = CompiledTemplate::compile(template).unwrap();
        let rendered = compiled.render_with_cursor(&context).unwrap().0;
        if !rendered.contains('\n') {
            assert_eq!(
                render_template_preview(template, &context, usize::MAX).unwrap(),
                rendered,
                "{template}"
            );
        }
        let streamed = render_nested(
            template,
            &context,
            false,
            0,
            MAX_RENDERED_BYTES,
            false,
            &mut HashMap::new(),
        )
        .unwrap()
        .0;
        assert_eq!(
            compiled
                .render(&context, false, MAX_RENDERED_BYTES)
                .unwrap()
                .0,
            streamed,
            "{template}"
        );
    }
}

#[test]
fn compiled_templates_reject_static_errors_and_keep_runtime_ones_for_render() {
    for (template, expected) in [
        ("{{nope}}", "unknown"),
        ("{{", "unclosed"),
        ("{{ }}", "empty"),
        ("{{uuid:}}", "uuid"),
        ("{{shoe+1d}}", "unknown"),
    ] {
        let error = CompiledTemplate::compile(template).unwrap_err();
        let matched = match expected {
            "unknown" => matches!(error, TemplateError::UnknownVariable { .. }),
            "unclosed" => matches!(error, TemplateError::Unclosed { .. }),
            "empty" => matches!(error, TemplateError::EmptyVariable { .. }),
            _ => matches!(error, TemplateError::InvalidUuidName),
        };
        assert!(matched, "{template}: {error:?}");
    }
    // These depend on the render context, so they compile and fail at
    // render time exactly as before.
    let context = TemplateContext::default();
    for template in ["{{env:UNSET}}", "{{snippet:missing}}", "{{clipboard}}"] {
        let compiled = CompiledTemplate::compile(template).unwrap();
        assert!(compiled.render_with_cursor(&context).is_err(), "{template}");
    }
    let cursor = CompiledTemplate::compile("a{{cursor}}b{{cursor}}").unwrap();
    assert!(matches!(
        cursor.render_with_cursor(&context),
        Err(TemplateError::MultipleCursors { offset: 12 })
    ));
    assert!(matches!(
        render_template("a{{cursor}}b", &context),
        Err(TemplateError::UnknownVariable { .. })
    ));
    let one = CompiledTemplate::compile("<{{cursor}}>").unwrap();
    assert_eq!(
        one.render_with_cursor(&context).unwrap(),
        ("<>".to_owned(), Some(1))
    );
    // A named UUID is one value within a render and fresh across renders.
    let named = CompiledTemplate::compile("{{uuid:a}} {{uuid:a}}").unwrap();
    let first = named.render_with_cursor(&context).unwrap().0;
    let (left, right) = first.split_once(' ').unwrap();
    assert_eq!(left, right);
    assert_ne!(named.render_with_cursor(&context).unwrap().0, first);
}

#[test]
fn renders_builtins_without_shelling_out() {
    let context = TemplateContext {
        username: "ada".into(),
        hostname: "workstation".into(),
        unix_timestamp: 0,
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template(
            "Hi {{username}} on {{hostname}} {{date}} {{time}}{{newline}}",
            &context
        )
        .unwrap(),
        "Hi ada on workstation 1970-01-01 00:00:00\n"
    );
}

#[test]
fn local_clock_variables_apply_the_utc_offset() {
    // 2024-01-01T00:30:00Z
    let timestamp = 1_704_069_000;
    assert_eq!(
        format_clock_with_offset("datetime", timestamp, None).as_deref(),
        Some("2024-01-01T00:30:00Z")
    );
    // West of UTC the local calendar day is still the previous one.
    assert_eq!(
        format_clock_with_offset("date", timestamp, Some(-5 * 3_600)).as_deref(),
        Some("2023-12-31")
    );
    assert_eq!(
        format_clock_with_offset("datetime", timestamp, Some(-5 * 3_600)).as_deref(),
        Some("2023-12-31T19:30:00-05:00")
    );
    assert_eq!(
        format_clock_with_offset("datetime", timestamp, Some(5 * 3_600 + 45 * 60)).as_deref(),
        Some("2024-01-01T06:15:00+05:45")
    );
    // Validation renders at the epoch; a western zone must still format.
    assert_eq!(
        format_clock_with_offset("datetime", 0, Some(-8 * 3_600)).as_deref(),
        Some("1969-12-31T16:00:00-08:00")
    );
}

#[test]
fn local_clock_variables_render_with_the_system_offset() {
    let context = TemplateContext {
        unix_timestamp: 1_704_069_000,
        ..TemplateContext::default()
    };
    let offset = local_utc_offset(context.unix_timestamp).unwrap();
    assert_eq!(
        render_template("{{local_datetime}}", &context).unwrap(),
        format_clock_with_offset("datetime", context.unix_timestamp, Some(offset)).unwrap()
    );
    let tomorrow = context.unix_timestamp + 86_400;
    assert_eq!(
        render_template("{{local_date+1d}}", &context).unwrap(),
        format_clock_with_offset("date", tomorrow, Some(local_utc_offset(tomorrow).unwrap()))
            .unwrap()
    );
    for template in ["{{local_date}}", "{{local_time}}", "{{local_datetime-1w}}"] {
        render_template(template, &context).unwrap();
    }
}

#[test]
fn rejects_unknown_and_unclosed_variables() {
    let context = TemplateContext::default();
    assert!(matches!(
        render_template("{{secret}}", &context),
        Err(TemplateError::UnknownVariable { .. })
    ));
    assert!(matches!(
        render_template("{{date", &context),
        Err(TemplateError::Unclosed { .. })
    ));
}

#[test]
fn cursor_marker_accepts_whitespace_and_rejects_a_second_marker() {
    let context = TemplateContext::default();
    assert_eq!(
        render_template_with_cursor("a {{ cursor }}bc", &context).unwrap(),
        ("a bc".to_owned(), Some(2))
    );
    assert_eq!(
        render_template_with_cursor("{{cursor}}{{newline}}x", &context).unwrap(),
        ("\nx".to_owned(), Some(2))
    );
    assert!(matches!(
        render_template_with_cursor("v{{cursor}} and v{{cursor}}", &context),
        Err(TemplateError::MultipleCursors { offset: 17 })
    ));
    // Plain rendering has no cursor to place, so the marker stays unknown.
    assert!(matches!(
        render_template("{{cursor}}", &context),
        Err(TemplateError::UnknownVariable { .. })
    ));
}

#[test]
fn uses_long_year_safe_calendar_conversion() {
    let context = TemplateContext {
        unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template("{{datetime}}", &context).unwrap(),
        "2024-01-01T00:00:00Z"
    );
}

#[test]
fn date_math_supports_day_and_week_offsets() {
    let context = TemplateContext {
        unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z (a Monday)
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template("{{date+3d}}", &context).unwrap(),
        "2024-01-04"
    );
    assert_eq!(
        render_template("{{date-1d}}", &context).unwrap(),
        "2023-12-31"
    );
    assert_eq!(
        render_template("{{date+1w}}", &context).unwrap(),
        "2024-01-08"
    );
}

#[test]
fn date_math_supports_hour_and_minute_offsets_on_time_and_datetime() {
    let context = TemplateContext {
        unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template("{{time+5h}}", &context).unwrap(),
        "05:00:00"
    );
    assert_eq!(
        render_template("{{datetime+90m}}", &context).unwrap(),
        "2024-01-01T01:30:00Z"
    );
}

#[test]
fn date_math_offset_crossing_a_year_boundary_is_calendar_correct() {
    let context = TemplateContext {
        unix_timestamp: 1_704_067_200, // 2024-01-01T00:00:00Z
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template("{{date-1d}}", &context).unwrap(),
        "2023-12-31"
    );
}

#[test]
fn date_math_rejects_malformed_or_overflowing_offsets_as_unknown_variables() {
    let context = TemplateContext::default();
    for template in [
        "{{date+3x}}",
        "{{date+}}",
        "{{date+99999999999999999999d}}",
        "{{date+999999999999999999w}}",
    ] {
        assert!(matches!(
            render_template(template, &context),
            Err(TemplateError::UnknownVariable { .. })
        ));
    }
}

#[test]
fn cursor_offset_counts_combining_sequence_as_one_grapheme() {
    let (rendered, offset) =
        render_template_with_cursor("prefix{{cursor}}e\u{301}", &TemplateContext::default())
            .unwrap();

    assert_eq!(rendered, "prefixe\u{301}");
    assert_eq!(offset, Some(1));
}

#[test]
fn cursor_offset_counts_zwj_emoji_as_one_grapheme() {
    let family = "👨‍👩‍👧‍👦";
    let (rendered, offset) = render_template_with_cursor(
        &format!("prefix{{{{cursor}}}}{family}"),
        &TemplateContext::default(),
    )
    .unwrap();

    assert_eq!(rendered, format!("prefix{family}"));
    assert_eq!(offset, Some(1));
}

#[test]
fn cursor_offset_counts_emoji_modifier_as_one_grapheme() {
    let (rendered, offset) =
        render_template_with_cursor("prefix{{cursor}}👍🏽", &TemplateContext::default()).unwrap();

    assert_eq!(rendered, "prefix👍🏽");
    assert_eq!(offset, Some(1));
}

#[test]
fn doubled_braces_render_literal_template_syntax() {
    let context = TemplateContext::default();
    let playbook = r#"name: "{{{{ item }}}}" on {{tab}}{{{{ansible_host}}}}"#;
    assert_eq!(
        render_template(playbook, &context).unwrap(),
        "name: \"{{ item }}\" on \t{{ansible_host}}"
    );
    // An escaped opener is not a marker, so nothing inside it is parsed.
    assert!(template_variables(r"{{{{ cursor }}}} {{tab}}") == vec!["tab"]);
    assert_eq!(
        render_template_with_cursor(r"{{{{ x }}}}{{cursor}}!", &context).unwrap(),
        ("{{ x }}!".to_owned(), Some(1))
    );
    // A lone `}}` in literal text is unchanged, as before.
    assert_eq!(render_template("a }} b", &context).unwrap(), "a }} b");
    assert_eq!(
        render_template_preview(r"{{{{ item }}}}", &context, 3).unwrap(),
        "{{ ",
        "previews stay within their budget"
    );
}

#[test]
fn escaped_braces_mix_with_form_fields() {
    let template = r#"- hosts: {{field:hosts=all}}
  tasks:
    - ansible.builtin.package:
        name: "{{{{ item }}}}"
        state: {{choice:state=present|latest|absent}}
      loop: [{{field:package}}]"#;
    let fields = form_fields(template).unwrap();
    let labels: Vec<&str> = fields.iter().map(|field| field.label.as_str()).collect();
    assert_eq!(labels, ["hosts", "state", "package"]);
    let mut values = HashMap::new();
    values.insert("field:hosts".to_owned(), "web".to_owned());
    values.insert("choice:state".to_owned(), "latest".to_owned());
    values.insert("field:package".to_owned(), "nginx".to_owned());
    let context = TemplateContext {
        fields: Arc::new(values.into()),
        ..TemplateContext::default()
    };
    assert_eq!(
        render_template(template, &context).unwrap(),
        r#"- hosts: web
  tasks:
    - ansible.builtin.package:
        name: "{{ item }}"
        state: latest
      loop: [nginx]"#
    );
}

#[test]
fn shell_quoted_form_values_are_discovered_and_cannot_break_out_of_the_shell_word() {
    let template = "curl {{shellq:domain=example.net|required|max=253}} {{shellq:domain}}";
    let fields = form_fields(template).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, "field:domain");
    assert!(fields[0].required);

    let values = [(
        "field:domain".to_owned(),
        "example.net'; touch /tmp/should-not-run; echo '".to_owned(),
    )]
    .into_iter()
    .collect();
    let context = TemplateContext {
        fields: Arc::new(values),
        ..TemplateContext::default()
    };
    assert_eq!(
            render_template(template, &context).unwrap(),
            "curl 'example.net'\\''; touch /tmp/should-not-run; echo '\\''' 'example.net'\\''; touch /tmp/should-not-run; echo '\\'''"
        );

    let values = [("field:domain".to_owned(), "example.net\nwhoami".to_owned())]
        .into_iter()
        .collect();
    let context = TemplateContext {
        fields: Arc::new(values),
        ..TemplateContext::default()
    };
    assert!(matches!(
        render_template(template, &context),
        Err(TemplateError::InvalidField)
    ));
}

#[test]
fn typed_infrastructure_fields_validate_names_ports_and_syntax_safe_values() {
    let fields = form_fields(
            "{{field:domain=example.com|required|validate=hostname}} {{field:service=api|required|validate=dns-label}} {{field:endpoint=127.0.0.1:8080|required|validate=host-port}} {{field:port=443|required|validate=port}} {{field:replicas=2|required|validate=positive-integer}} {{field:image=nginx:stable|required|validate=container-image}} {{field:description=API service|required|validate=single-line}} {{field:yaml=nginx|required|validate=yaml-string}} {{field:shell=nginx.service|required|validate=shell-single-quoted}} {{field:unit=api@blue.service|required|validate=systemd-unit}}",
        )
        .unwrap();
    let accepted = [
        "api.example.com",
        "api-1",
        "127.0.0.1:8080",
        "443",
        "2",
        "registry.example/team/api:v2",
        "API service",
        "nginx",
        "nginx.service",
        "api@blue.service",
    ];
    for (field, value) in fields.iter().zip(accepted) {
        assert!(
            field.validate_value(value).is_ok(),
            "{}: {value}",
            field.label
        );
    }

    for (field, value) in fields.iter().zip([
        "bad host",
        "UPPER_case",
        "127.0.0.1:99999",
        "0",
        "two",
        "nginx; touch /tmp/pwn",
        "two\nlines",
        "bad\"\nkey: value",
        "nginx.service'; reboot; #",
        "-oProxyCommand=touch /tmp/pwn",
    ]) {
        assert!(
            field.validate_value(value).is_err(),
            "{}: {value:?}",
            field.label
        );
    }
}

#[test]
fn named_choices_have_a_label_and_a_stable_key() {
    let fields =
        form_fields("{{choice:env=dev|prod}} {{choice:env=dev|prod}} {{choice:a|b}}").unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].label, "env");
    assert_eq!(fields[0].key, "choice:env");
    assert_eq!(
        fields[0].kind,
        FormFieldKind::Choice {
            options: vec!["dev".to_owned(), "prod".to_owned()],
            default: None,
        }
    );
    assert_eq!(fields[1].label, "Choice");
    assert_eq!(fields[1].key, "choice:a|b");
    // `=` after the first option is part of an option, as before.
    assert_eq!(
        form_fields("{{choice:a|b=c}}").unwrap()[0].kind,
        FormFieldKind::Choice {
            options: vec!["a".to_owned(), "b=c".to_owned()],
            default: None,
        }
    );
    for invalid in [
        "{{choice:=a|b}}",
        "{{choice:env=a}}",
        "{{choice:env=dev|prod}} {{choice:env=dev|test}}",
    ] {
        assert_eq!(
            form_fields(invalid),
            Err(TemplateError::InvalidField),
            "{invalid}"
        );
    }
}

#[test]
fn bare_references_reuse_the_first_named_form_field_definition() {
    let fields = form_fields(
            "{{field:host=example.net|required|min=3}} {{field:host}} {{prompt:host}} {{datefield:reported=2026-10-28|required}} {{datefield:reported}}",
        )
        .unwrap();

    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].key, "field:host");
    assert_eq!(
        fields[0].kind,
        FormFieldKind::Text {
            default: "example.net".into()
        }
    );
    assert!(fields[0].required);
    assert_eq!(fields[0].min_chars, Some(3));
    assert_eq!(fields[1].key, "date:reported");
    assert_eq!(
        fields[1].kind,
        FormFieldKind::Date {
            default: "2026-10-28".into()
        }
    );
    assert!(fields[1].required);
}

#[test]
fn a_definition_after_bare_references_supplies_the_field_rules() {
    let fields =
        form_fields("Hi {{field:host}}, {{field:host=example.net|required}} {{field:host}}")
            .unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(
        fields[0].kind,
        FormFieldKind::Text {
            default: "example.net".into()
        }
    );
    assert!(fields[0].required);
    assert_eq!(
        form_fields("{{field:host}} {{field:host=a}} {{field:host=b}}"),
        Err(TemplateError::InvalidField)
    );
}

#[test]
fn incident_form_parses_defaults_and_validates_values() {
    let template = "{{datefield:reported_at=today|required}} {{field:hostname|required|max=253}} {{choice:severity=Low|Medium|High|Critical;default=High;required}} {{field:summary|required|min=10|max=2000}} {{field:resolution|required|min=3|max=4000}}";
    let fields = form_fields(template).unwrap();
    assert_eq!(fields.len(), 5);
    assert!(matches!(fields[0].kind, FormFieldKind::Date { ref default } if default == "today"));
    assert_eq!(fields[2].key, "choice:severity");
    assert_eq!(
        fields[2].kind,
        FormFieldKind::Choice {
            options: vec![
                "Low".into(),
                "Medium".into(),
                "High".into(),
                "Critical".into()
            ],
            default: Some("High".into()),
        }
    );
    assert_eq!(fields[0].validate_value("2024-02-29"), Ok(()));
    assert_eq!(
        fields[0].validate_value("2025-02-29"),
        Err(FormValueError::InvalidDate)
    );
    assert_eq!(
        fields[1].validate_value("  "),
        Err(FormValueError::Required)
    );
    assert_eq!(
        fields[1].validate_value(&"h".repeat(254)),
        Err(FormValueError::TooLong { maximum: 253 })
    );
    assert_eq!(
        fields[3].validate_value("short"),
        Err(FormValueError::TooShort { minimum: 10 })
    );
    assert_eq!(
        fields[2].validate_value("Urgent"),
        Err(FormValueError::InvalidChoice)
    );
}

#[test]
fn invalid_form_rules_and_choice_defaults_are_rejected() {
    for template in [
        "{{datefield:when=2025-02-29}}",
        "{{field:name|min=10|max=2}}",
        "{{choice:severity=Low|High;default=Medium}}",
    ] {
        assert_eq!(
            form_fields(template),
            Err(TemplateError::InvalidField),
            "{template}"
        );
    }
}

#[test]
fn formatted_dates_apply_to_every_clock_variable_and_offset() {
    let context = TemplateContext {
        // 2026-10-09 14:05:07 UTC, a Friday.
        unix_timestamp: 1_791_554_707,
        ..TemplateContext::default()
    };
    let render = |template: &str| render_template(template, &context).unwrap();
    assert_eq!(render("{{date:%A %-d %B}}"), "Friday 9 October");
    assert_eq!(render("{{time:%H:%M}}"), "14:05");
    assert_eq!(render("{{date+1d:%a %F}}"), "Sat 2026-10-10");
    assert_eq!(render("{{datetime-90m:%H:%M %Z}}"), "12:35 UTC");
    assert_eq!(render("{{ date : %Y }}"), " 2026");
    // Local time: the format sees the system offset at that instant.
    let local = render("{{local_date:%F %H:%M %:z}}");
    assert_eq!(local.len(), "2026-10-09 08:05 -06:00".len(), "{local}");
    assert!(matches!(
        render_template("{{date:%Q}}", &context),
        Err(TemplateError::InvalidDateFormat { .. })
    ));
    // Not a clock: still an unknown variable.
    assert!(matches!(
        render_template("{{when:%F}}", &context),
        Err(TemplateError::UnknownVariable { .. })
    ));
}

#[test]
fn uuid_and_random_render_fresh_values_and_named_uuids_repeat() {
    let context = TemplateContext::default();
    let rendered = render_template("{{uuid}} {{uuid}} {{uuid:id}} {{uuid:id}}", &context).unwrap();
    let parts: Vec<&str> = rendered.split(' ').collect();
    assert_ne!(parts[0], parts[1]);
    assert_eq!(parts[2], parts[3]);
    assert_ne!(parts[2], parts[0]);
    // A named UUID is shared with included snippets in the same render.
    let mut snippets = HashMap::new();
    snippets.insert(":inner".to_owned(), Arc::<str>::from("{{uuid:id}}"));
    let context = TemplateContext {
        snippets: Arc::new(snippets),
        ..TemplateContext::default()
    };
    let rendered = render_template("{{uuid:id}}={{snippet::inner}}", &context).unwrap();
    let (outer, inner) = rendered.split_once('=').unwrap();
    assert_eq!(outer, inner);
    // ...but each render gets new values.
    assert_ne!(
        rendered,
        render_template("{{uuid:id}}={{snippet::inner}}", &context).unwrap()
    );

    let token = render_template("{{random:hex:16}}", &context).unwrap();
    assert_eq!(token.len(), 16);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    let pick = render_template("{{random:a|b}}", &context).unwrap();
    assert!(pick == "a" || pick == "b");
    for invalid in ["{{random:hex}}", "{{random:x:1}}", "{{uuid:}}"] {
        assert!(render_template(invalid, &context).is_err(), "{invalid}");
    }
}

#[test]
fn malformed_dynamic_markers_fail_configuration_validation() {
    for replacement in ["{{date:%Q}}", "{{random:hex:0}}", "{{uuid: }}"] {
        let text = format!("[[expansion]]\ntrigger = \":x\"\nreplacement = {replacement:?}\n");
        assert!(crate::Config::parse(&text).is_err(), "{replacement}");
    }
    let text = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{{local_date+1w:%A}} {{uuid}} {{random:password:20}}\"\n";
    assert!(crate::Config::parse(text).is_ok());
}

/// A long-running daemon must follow a change of the system time zone.
/// Changing `TZ` is only safe without other threads, so the check runs
/// in a child copy of this test binary limited to this one test.
#[test]
fn local_offset_follows_a_changed_time_zone() {
    const CHILD: &str = "WAYEXPAND_TZ_CHANGE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        // SAFETY: the parent runs this child with --test-threads=1 and
        // only this test, so no other thread reads the environment.
        unsafe { std::env::set_var("TZ", "AAA+03") };
        let before = super::local_utc_offset(1_800_000_000);
        unsafe { std::env::set_var("TZ", "BBB-05") };
        let after = super::local_utc_offset(1_800_000_000);
        assert_eq!(before, Some(-3 * 3_600));
        assert_eq!(after, Some(5 * 3_600));
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "template::tests::local_offset_follows_a_changed_time_zone",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The filter must have matched: a renamed test would pass vacuously.
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
