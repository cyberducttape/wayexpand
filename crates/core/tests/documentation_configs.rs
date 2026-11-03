use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use wayexpand_core::{
    render_template, Config, FormFieldKind, FormFieldValidation, FormValues, TemplateContext,
};

#[test]
fn published_executable_toml_examples_parse_as_config() {
    let documentation = include_str!("../../../docs/CERTIFICATION_MATRIX.md");
    let marker = "<!-- executable-toml: config -->";
    let mut examples = 0;
    let mut remainder = documentation;

    while let Some(marker_offset) = remainder.find(marker) {
        remainder = &remainder[marker_offset + marker.len()..];
        let fence = remainder
            .find("```toml\n")
            .expect("executable TOML marker must precede a TOML fence");
        let source = &remainder[fence + "```toml\n".len()..];
        let end = source
            .find("\n```")
            .expect("executable TOML fence must be closed");
        Config::parse(&source[..end]).expect("published TOML example must load as Config");
        examples += 1;
        remainder = &source[end + "\n```".len()..];
    }

    assert!(
        examples > 0,
        "documentation must publish executable TOML examples"
    );
}

#[test]
fn sysadmin_diagnostic_pack_is_valid_and_insert_only() {
    let source = include_str!("../../../packs/sysadmin-diagnostics/snippets/diagnostics.toml");
    let config = Config::parse(source).expect("sysadmin diagnostic snippets should validate");
    let triggers: Vec<_> = config
        .expansion
        .iter()
        .map(|expansion| expansion.trigger.as_str())
        .collect();
    assert_eq!(
        triggers,
        [
            ";tlscheck",
            ";svcdiag",
            ";portcheck",
            ";httpcheck",
            ";pathcheck",
            ";sshcheck"
        ]
    );
    assert!(config.expansion.iter().all(|expansion| {
        expansion.command.is_none()
            && wayexpand_core::form_fields(&expansion.replacement)
                .is_ok_and(|fields| !fields.is_empty())
    }));

    for (trigger, key, accepted, rejected) in [
        (
            ";tlscheck",
            "field:domain",
            "api.example.net",
            "-oProxyCommand=touch /tmp/pwn",
        ),
        (";tlscheck", "field:port", "443", "22;id"),
        (
            ";svcdiag",
            "field:service",
            "nginx.service",
            "-oProxyCommand=touch /tmp/pwn",
        ),
        (
            ";sshcheck",
            "field:user",
            "admin",
            "-oProxyCommand=touch /tmp/pwn",
        ),
        (
            ";sshcheck",
            "field:host",
            "host.example.net",
            "-oProxyCommand=touch /tmp/pwn",
        ),
    ] {
        let expansion = config
            .expansion
            .iter()
            .find(|expansion| {
                expansion.trigger == trigger
                    || expansion.aliases.iter().any(|alias| alias == trigger)
            })
            .expect("the expected diagnostic expansion should exist");
        let field = wayexpand_core::form_fields(&expansion.replacement)
            .unwrap()
            .into_iter()
            .find(|field| field.key == key)
            .expect("the expected validated field should exist");
        assert!(field.validate_value(accepted).is_ok(), "{trigger} {key}");
        assert!(field.validate_value(rejected).is_err(), "{trigger} {key}");
    }
}

#[test]
fn curated_operations_packs_validate_command_arguments() {
    let kubernetes = Config::parse(include_str!(
        "../../../packs/kubernetes/snippets/kubectl.toml"
    ))
    .expect("Kubernetes operations pack should validate");
    let linux = Config::parse(include_str!(
        "../../../packs/linux-administration/snippets/operations.toml"
    ))
    .expect("Linux administration pack should validate");

    for (config, triggers) in [
        (
            &kubernetes,
            [
                ";;kube-pods",
                ";;kube-logs",
                ";;kube-events",
                ";;kube-rollout",
            ]
            .as_slice(),
        ),
        (
            &linux,
            [
                ";;journal-service",
                ";;service-status",
                ";;ssh-check",
                ";;bash-safe",
            ]
            .as_slice(),
        ),
    ] {
        for trigger in triggers {
            let expansion = config
                .expansion
                .iter()
                .find(|expansion| {
                    expansion.trigger == *trigger
                        || expansion.aliases.iter().any(|alias| alias == *trigger)
                })
                .unwrap_or_else(|| panic!("missing curated shell command {trigger}"));
            assert!(
                !expansion.replacement.contains("{{field:"),
                "shell arguments in {trigger} must use the shell-quoting variable"
            );
        }
    }

    for expansion in kubernetes.expansion.iter().chain(&linux.expansion) {
        assert!(
            expansion.command.is_none(),
            "{} must remain insert-only",
            expansion.trigger
        );
        let fields = wayexpand_core::form_fields(&expansion.replacement)
            .expect("curated form markers should parse");
        for field in fields {
            if let FormFieldKind::Text { default } = &field.kind {
                let validation = field.validate_value(default);
                if field.required && default.is_empty() {
                    assert!(
                        validation.is_err(),
                        "required field in {} unexpectedly accepts an empty default: {}",
                        expansion.trigger,
                        field.label
                    );
                } else {
                    assert!(
                        validation.is_ok(),
                        "invalid default in {}: {}",
                        expansion.trigger,
                        field.label
                    );
                }
            }
        }
    }

    for (config, trigger, field_key, validation) in [
        (
            &kubernetes,
            ";;kube-pods",
            "field:namespace",
            FormFieldValidation::DnsLabel,
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:namespace",
            FormFieldValidation::DnsLabel,
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:pod",
            FormFieldValidation::KubernetesName,
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:container",
            FormFieldValidation::DnsLabel,
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:since",
            FormFieldValidation::KubernetesDuration,
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:lines",
            FormFieldValidation::PositiveInteger,
        ),
        (
            &kubernetes,
            ";;kube-events",
            "field:namespace",
            FormFieldValidation::DnsLabel,
        ),
        (
            &kubernetes,
            ";;kube-rollout",
            "field:deployment",
            FormFieldValidation::KubernetesName,
        ),
        (
            &kubernetes,
            ";;kube-rollout",
            "field:namespace",
            FormFieldValidation::DnsLabel,
        ),
        (
            &kubernetes,
            ";;kube-rollout",
            "field:timeout",
            FormFieldValidation::Duration,
        ),
        (
            &linux,
            ";;journal-service",
            "field:unit",
            FormFieldValidation::SystemdUnit,
        ),
        (
            &linux,
            ";;journal-service",
            "field:since",
            FormFieldValidation::SingleLine,
        ),
        (
            &linux,
            ";;journal-service",
            "field:lines",
            FormFieldValidation::PositiveInteger,
        ),
        (
            &linux,
            ";;service-status",
            "field:unit",
            FormFieldValidation::SystemdUnit,
        ),
        (
            &linux,
            ";;ssh-check",
            "field:user",
            FormFieldValidation::LinuxUser,
        ),
        (
            &linux,
            ";;ssh-check",
            "field:host",
            FormFieldValidation::Hostname,
        ),
        (
            &linux,
            ";;bash-safe",
            "field:target",
            FormFieldValidation::AbsolutePath,
        ),
        (
            &linux,
            ";;systemd-unit",
            "field:executable",
            FormFieldValidation::SystemdExecutable,
        ),
        (
            &linux,
            ";;systemd-unit",
            "field:description",
            FormFieldValidation::SingleLine,
        ),
    ] {
        let expansion = config
            .expansion
            .iter()
            .find(|expansion| {
                expansion.trigger == trigger
                    || expansion.aliases.iter().any(|alias| alias == trigger)
            })
            .unwrap_or_else(|| panic!("missing curated snippet {trigger}"));
        let field = wayexpand_core::form_fields(&expansion.replacement)
            .unwrap()
            .into_iter()
            .find(|field| field.key == field_key)
            .unwrap_or_else(|| panic!("missing {field_key} in {trigger}"));
        assert_eq!(field.validation, Some(validation), "{trigger}: {field_key}");
    }

    for (config, trigger, field_key, hostile_values) in [
        (
            &kubernetes,
            ";;kube-pods",
            "field:namespace",
            ["team space", "team;id", "team'ops", "生产"],
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:pod",
            ["pod name", "pod;id", "pod'ops", "поды"],
        ),
        (
            &kubernetes,
            ";;kube-logs",
            "field:lines",
            ["10 lines", "10;id", "１０", "-1"],
        ),
        (
            &kubernetes,
            ";;kube-rollout",
            "field:timeout",
            ["120 seconds", "120s;id", "１２０s", "0s"],
        ),
        (
            &linux,
            ";;ssh-check",
            "field:user",
            ["admin ops", "admin';id", "-root", "üser"],
        ),
        (
            &linux,
            ";;ssh-check",
            "field:host",
            ["host name", "host';id", "-oProxyCommand=x", "täst.example"],
        ),
        (
            &linux,
            ";;service-status",
            "field:unit",
            ["bad unit", "api.service;id", "api'ops", "сервис"],
        ),
        (
            &linux,
            ";;systemd-unit",
            "field:executable",
            [
                "relative/path",
                "/usr/bin/foo bar",
                "/usr/bin/foo\";id",
                "/usr/bin/foo\\bar",
            ],
        ),
    ] {
        let expansion = config
            .expansion
            .iter()
            .find(|expansion| {
                expansion.trigger == trigger
                    || expansion.aliases.iter().any(|alias| alias == trigger)
            })
            .unwrap();
        let field = wayexpand_core::form_fields(&expansion.replacement)
            .unwrap()
            .into_iter()
            .find(|field| field.key == field_key)
            .unwrap();
        for hostile in hostile_values {
            assert!(
                field.validate_value(hostile).is_err(),
                "{trigger} accepted {hostile:?} for {field_key}"
            );
        }
    }

    let journal = linux
        .expansion
        .iter()
        .find(|expansion| expansion.trigger == ";;journal-service")
        .unwrap();
    let mut values = FormValues::default();
    for field in wayexpand_core::form_fields(&journal.replacement).unwrap() {
        let FormFieldKind::Text { default } = field.kind else {
            unreachable!("journal fields are text fields")
        };
        values.insert(field.key, default);
    }
    values.insert(
        "field:since".to_owned(),
        "1 hour'; touch /tmp/wayexpand-must-not-run; echo ' ago – café".to_owned(),
    );
    let rendered = render_template(
        &journal.replacement,
        &TemplateContext {
            fields: Arc::new(values),
            ..TemplateContext::default()
        },
    )
    .expect("quoted single-line value should render safely");
    assert!(rendered.contains(
        "--since '1 hour'\\''; touch /tmp/wayexpand-must-not-run; echo '\\'' ago – café'"
    ));
}

#[test]
fn ensure_user_config_creates_a_private_config_on_first_run() {
    let root = std::env::temp_dir().join(format!(
        "wayexpand-first-run-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos()
    ));
    let path = root.join("config/wayexpand/expansions.toml");
    std::fs::create_dir_all(root.join("config")).expect("test config home should exist");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("test home should be private");
    std::fs::set_permissions(root.join("config"), std::fs::Permissions::from_mode(0o700))
        .expect("test config home should be private");

    let loaded = Config::ensure_user_config(&path).expect("first-run config should be created");
    assert!(loaded.config.expansion.is_empty());
    assert_eq!(
        std::fs::metadata(&path)
            .expect("first-run config should exist")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(Config::load(&path).is_ok());

    std::fs::remove_dir_all(root).expect("test config directory should be removable");
}
