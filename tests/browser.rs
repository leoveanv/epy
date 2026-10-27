use ers::integrations::BrowserCommand;
use url::Url;

#[test]
fn browser_commands_preserve_quoted_arguments() {
    let url = Url::parse("https://example.org/path?a=1&b=2").unwrap();
    let command = BrowserCommand::parse("browser --profile 'reading profile' '%s'", &url).unwrap();
    assert_eq!(command.program, "browser");
    assert_eq!(
        command.arguments,
        ["--profile", "reading profile", url.as_str()]
    );
}

#[test]
fn browser_commands_append_urls_without_placeholders() {
    let url = Url::parse("https://example.org/path").unwrap();
    let command = BrowserCommand::parse("browser --new-window", &url).unwrap();
    assert_eq!(command.arguments, ["--new-window", url.as_str()]);
}

#[test]
fn browser_commands_do_not_interpret_shell_metacharacters() {
    let url = Url::parse("https://example.org/path?q=$(touch%20/tmp/forbidden);next").unwrap();
    let command = BrowserCommand::parse("browser %s", &url).unwrap();
    assert_eq!(command.arguments, [url.as_str()]);
}

#[test]
fn browser_commands_reject_unsafe_protocols_before_launch() {
    let url = Url::parse("file:///etc/passwd").unwrap();
    assert!(BrowserCommand::parse("browser %s", &url).is_err());
}
