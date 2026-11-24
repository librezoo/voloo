use std::process::Command;

#[test]
fn shell_setup_and_contextual_suggestions_work_offline() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
            .env("VOLOO_HOME", home.path())
            .env("VOLOO_SCAN_USB", "0")
            .env_remove("VOLOO_LOG_LEVEL")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };

    for (shell, registration) in [
        ("bash", "complete -o"),
        ("zsh", "compdef"),
        ("fish", "complete --exclusive"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let script = run(&["completions", shell]);
        assert!(script.contains(registration) && script.contains("__complete"));
    }
    assert_eq!(run(&["__complete", "=in"]), "info\n");
    assert_eq!(run(&["__complete", "=ins", "firmware"]), "inspect\n");
    assert_eq!(run(&["__complete", "=a", "diag", "--kind"]), "adc\n");
    assert_eq!(run(&["__complete", "=--kind=a", "diag"]), "--kind=adc\n");
    assert_eq!(
        run(&["__complete", "=tr", "info", "--log-level"]),
        "trace\n"
    );
    assert!(!run(&["__complete", "="]).contains("__complete"));
    assert!(!home.path().join("daemon.json").exists());
}
