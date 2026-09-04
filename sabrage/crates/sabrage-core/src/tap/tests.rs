use super::*;

#[test]
fn words_match_the_zsh_vocabulary() {
    assert_eq!(tap_word(CheckStatus::Pass), "ok");
    assert_eq!(tap_word(CheckStatus::Warn), "warn");
    assert_eq!(tap_word(CheckStatus::Fail), "fail");
    assert_eq!(tap_word(CheckStatus::Info), "info");
    assert_eq!(tap_word(CheckStatus::Skipped), "skipped");
    assert_eq!(tap_word(CheckStatus::NotImplemented), "skipped");
}

#[test]
fn renders_one_line_per_outcome() {
    let outcomes = vec![
        CheckOutcome::pass("sys.arch", "Apple Silicon"),
        CheckOutcome::fail(
            "cx.version",
            "CrossOver 26.1 < 26.2",
            "upgrade CrossOver to 26.2+",
        ),
        CheckOutcome::skipped("hs.client", "no adb device".into()),
    ];
    assert_eq!(
        render_tap(&outcomes),
        "sys.arch ok\ncx.version fail\nhs.client skipped\n"
    );
}
