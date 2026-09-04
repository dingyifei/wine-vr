use super::*;

#[test]
fn registry_binds_in_contract_order_and_covers_every_slug() {
    let reg = registry();
    assert_eq!(reg.len(), contract().checks.len());
    let slugs: Vec<&str> = reg.checks().iter().map(|c| c.slug()).collect();
    assert_eq!(slugs, contract().check_slugs());
}

#[test]
fn complete_registry_builds_leniently() {
    // The lenient path is exercised nowhere else. The strict build and
    // "no contract slug is unbound" are pinned by sabrage-parity's
    // strict_registry_builds_and_covers_the_contract_in_order, the layer CI runs.
    assert!(build_registry(false).is_ok());
}

#[test]
fn doctor_walks_only_doctor_visible_checks() {
    // doctor.sh never evaluates or taps the run-only preflights; the
    // native doctor must emit exactly the doctor-visible subset, in
    // contract order, with no NotImplemented (all of them are bound).
    let ctx = CheckCtx::new(Paths::new("/nonexistent/repo"), CheckOptions::new());
    let reg = registry();
    let visible = reg.doctor_checks().count();
    let run_only = contract()
        .checks
        .iter()
        .filter(|c| c.group == NO_DOCTOR_ROW_GROUP)
        .count();
    assert!(run_only > 0, "the contract still declares run-only slugs");
    assert_eq!(visible, reg.len() - run_only);
    let mut seen = 0usize;
    let report = run_doctor_with(&reg, &ctx, &mut |_| seen += 1);
    assert_eq!(seen, visible);
    assert_eq!(report.outcomes.len(), visible);
    for o in &report.outcomes {
        assert_ne!(
            o.status,
            CheckStatus::NotImplemented,
            "doctor-visible slug {} has no evaluator",
            o.slug
        );
        assert_ne!(
            reg.get(&o.slug).unwrap().spec.group,
            NO_DOCTOR_ROW_GROUP,
            "run-only slug {} leaked into doctor output",
            o.slug
        );
    }
}

#[test]
fn native_preflight_is_the_gating_subset() {
    let reg = registry();
    let pre: Vec<&str> = reg.native_preflight().iter().map(|c| c.slug()).collect();
    let want: Vec<&str> = contract()
        .native_preflight()
        .iter()
        .map(|s| s.slug.as_str())
        .collect();
    assert_eq!(pre, want);
    assert!(pre.contains(&"run.wine-exec"));
    assert!(!pre.contains(&"sys.arch"));
}

#[test]
fn ctx_bottle_label_falls_back_to_the_doctor_placeholder() {
    let ctx = CheckCtx::new(Paths::new("/repo"), CheckOptions::new());
    assert_eq!(ctx.bottle_label(), "<name>");
    assert!(!ctx.bottle_requested);
    assert_eq!(ctx.prefix(), Path::new(""));

    let opts = CheckOptions {
        bottle_name: Some("NoSuchBottle".into()),
        ..CheckOptions::new()
    };
    let ctx = CheckCtx::new(Paths::new("/repo"), opts);
    assert_eq!(ctx.bottle_label(), "NoSuchBottle");
    assert!(ctx.bottle_requested);
    // Named but non-existent: requested, unresolved.
    assert!(ctx.bottle.is_none());
}
