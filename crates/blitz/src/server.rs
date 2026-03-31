pub async fn start_embedded(passphrase: &str) -> String {
    let sock_path = format!(
        "{}/blitz-{}.sock",
        std::env::var("TMPDIR")
            .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
            .unwrap_or_else(|_| "/tmp".into()),
        &passphrase[..8.min(passphrase.len())],
    );

    let config = blit_server::Config {
        shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        shell_flags: "li".into(),
        scrollback: 10_000,
        socket_path: sock_path.clone(),
        fd_channel: None,
    };

    let path = sock_path.clone();
    tokio::spawn(async move {
        blit_server::run(config).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let cleanup_path = sock_path.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = std::fs::remove_file(&cleanup_path);
    });

    path
}
