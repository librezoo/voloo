use std::{fs, process::Command};

#[test]
fn install_uses_shell_directories_and_preserves_profiles() {
    let home = tempfile::tempdir().unwrap();
    let zdot = home.path().join("zsh config");
    let xdg = home.path().join("xdg config");
    let cli = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_voloo"));
        cmd.env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("ZDOTDIR", &zdot)
            .env("XDG_CONFIG_HOME", &xdg)
            .env("VOLOO_HOME", home.path().join("voloo"))
            .env_remove("VOLOO_LOG_LEVEL");
        cmd
    };
    let install = |shell| {
        let out = cli()
            .args(["completions", "install", shell])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    let profile = home.path().join(".profile");
    let old = b"# keep existing profile, with no trailing newline";
    fs::write(&profile, old).unwrap();
    install("bash");
    let first = fs::read(&profile).unwrap();
    assert!(first.starts_with(old));
    assert!(!home.path().join(".bash_profile").exists());
    assert!(
        fs::read_to_string(home.path().join(".bashrc"))
            .unwrap()
            .contains("voloo completions bash")
    );
    install("bash");
    assert_eq!(fs::read(&profile).unwrap(), first);

    fs::create_dir(&zdot).unwrap();
    let zshrc = zdot.join(".zshrc");
    fs::write(&zshrc, "source <(voloo completions zsh)\n").unwrap();
    install("zsh");
    assert_eq!(
        fs::read_to_string(&zshrc).unwrap(),
        "source <(voloo completions zsh)\n"
    );
    fs::write(&zshrc, old).unwrap();
    let children: Vec<_> = (0..4)
        .map(|_| {
            cli()
                .args(["completions", "install", "zsh"])
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let zsh = fs::read_to_string(zshrc).unwrap();
    assert!(zsh.as_bytes().starts_with(old));
    assert_eq!(
        zsh.matches("source <(command voloo completions zsh)")
            .count(),
        1
    );

    install("fish");
    let fish = xdg.join("fish/completions/voloo.fish");
    assert!(
        fs::read_to_string(&fish)
            .unwrap()
            .contains("voloo completions fish")
    );
    fs::write(&fish, "# my own completion\n").unwrap();
    install("fish");
    assert_eq!(fs::read_to_string(fish).unwrap(), "# my own completion\n");
}

#[test]
fn shell_setup_and_contextual_suggestions_work_offline() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
            .env("VOLOO_HOME", home.path())
            .env("VOLOO_SCAN_USB", "0")
            .env("VOLOO_PORTS", "COM4=01-23-45-67-89-AB-CD-EF")
            .env_remove("VOLOO_LOG_LEVEL")
            .current_dir(home.path())
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
    let suggest = |prefix: &str, words: &[&str]| {
        let prefix = format!("={prefix}");
        run(&[&["__complete", prefix.as_str()], words].concat())
    };
    assert_eq!(suggest("-h", &[]), "-h\n");
    assert_eq!(suggest("-V", &[]), "-V\n");
    assert_eq!(suggest("in", &["completions"]), "install\n");
    assert_eq!(suggest("fi", &["completions", "install"]), "fish\n");
    assert_eq!(suggest("COM", &["info", "--port"]), "COM4\n");
    assert_eq!(suggest("--port=COM", &[]), "--port=COM4\n");
    assert_eq!(suggest("", &["cycle", "bench-a", "--off-ms"]), "1000\n");
    assert_eq!(suggest("", &["faults", "--limit"]), "50\n");
    assert_eq!(suggest("", &["log", "--limit"]), "20\n");
    assert_eq!(
        suggest("", &["demo", "--count"]),
        "1\n2\n3\n4\n5\n6\n7\n8\n"
    );

    let directory = home.path().join("my firmware");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("new release.voloofw"), []).unwrap();
    let fw_dir = format!("my firmware{}\n", std::path::MAIN_SEPARATOR);
    assert_eq!(suggest("my", &["firmware", "inspect"]), fw_dir);
    let file_prefix = format!("my firmware{}new", std::path::MAIN_SEPARATOR);
    let file = format!("{file_prefix} release.voloofw\n");
    assert_eq!(suggest(&file_prefix, &["firmware", "inspect"]), file);
    assert_eq!(
        suggest(&file_prefix, &["firmware", "update", "bench-a"]),
        file
    );
    assert_eq!(suggest("--al", &["firmware", "update"]), "--all\n");
    assert!(suggest("--al", &["firmware", "update", "bench-a"]).is_empty());
    assert!(suggest(&file_prefix, &["firmware", "update", "--all"]).is_empty());
    assert_eq!(
        suggest("tr", &["firmware", "update", "--all", "--log-level"]),
        "trace\n"
    );

    let logs = home.path().join("logs");
    fs::create_dir(&logs).unwrap();
    fs::write(
        logs.join("trace-20261125.jsonl"),
        "{\"trace_id\":\"t123-a-1\",\"uid\":\"01-23-45-67-89-AB-CD-EF\"}\n",
    )
    .unwrap();
    assert_eq!(suggest("t123", &["trace"]), "t123-a-1\n");
    assert_eq!(
        suggest("01-", &["log", "--uid"]),
        "01-23-45-67-89-AB-CD-EF\n"
    );

    assert!(!home.path().join("daemon.json").exists());
}
