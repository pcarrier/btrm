use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub name: String,
    pub kind: RemoteKind,
    pub autoconnect: bool,
}

#[derive(Clone, Debug)]
pub enum RemoteKind {
    Unix { socket: Option<String> },
    Tcp { address: String },
    Ssh { host: String },
    Share { passphrase: String, hub: Option<String> },
}

pub fn default_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("blit").join("remotes.conf");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("blit").join("remotes.conf");
    }
    PathBuf::from("/etc/blit/remotes.conf")
}

pub fn load_remotes(path: Option<&Path>) -> Vec<RemoteConfig> {
    let p = match path {
        Some(p) => p.to_path_buf(),
        None => default_config_path(),
    };
    let content = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => return vec![default_local()],
    };
    let mut remotes = Vec::new();
    let sections = parse_ini(&content);
    for (name, props) in sections {
        if let Some(remote) = section_to_remote(&name, &props) {
            remotes.push(remote);
        }
    }
    if remotes.is_empty() {
        remotes.push(default_local());
    }
    remotes
}

pub fn save_remotes(path: Option<&Path>, remotes: &[RemoteConfig]) {
    let p = match path {
        Some(p) => p.to_path_buf(),
        None => default_config_path(),
    };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for (i, r) in remotes.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("[{}]\n", r.name));
        match &r.kind {
            RemoteKind::Unix { socket } => {
                out.push_str("type = unix\n");
                if let Some(s) = socket {
                    out.push_str(&format!("socket = {s}\n"));
                }
            }
            RemoteKind::Tcp { address } => {
                out.push_str("type = tcp\n");
                out.push_str(&format!("address = {address}\n"));
            }
            RemoteKind::Ssh { host } => {
                out.push_str("type = ssh\n");
                out.push_str(&format!("host = {host}\n"));
            }
            RemoteKind::Share { passphrase, hub } => {
                out.push_str("type = share\n");
                out.push_str(&format!("passphrase = {passphrase}\n"));
                if let Some(h) = hub {
                    out.push_str(&format!("hub = {h}\n"));
                }
            }
        }
        if r.autoconnect {
            out.push_str("autoconnect = true\n");
        }
    }
    let _ = std::fs::write(&p, out);
}

fn default_local() -> RemoteConfig {
    RemoteConfig {
        name: "local".into(),
        kind: RemoteKind::Unix { socket: None },
        autoconnect: true,
    }
}

fn parse_ini(content: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_props: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(name) = current_name.take() {
                sections.push((name, std::mem::take(&mut current_props)));
            }
            current_name = Some(line[1..line.len() - 1].trim().to_string());
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_lowercase();
            let val = line[eq + 1..].trim().to_string();
            current_props.insert(key, val);
        }
    }
    if let Some(name) = current_name {
        sections.push((name, current_props));
    }
    sections
}

fn section_to_remote(name: &str, props: &HashMap<String, String>) -> Option<RemoteConfig> {
    let kind_str = props.get("type").map(|s| s.as_str()).unwrap_or("unix");
    let autoconnect = props
        .get("autoconnect")
        .map(|v| v == "true" || v == "1" || v == "yes")
        .unwrap_or(false);
    let kind = match kind_str {
        "unix" => RemoteKind::Unix {
            socket: props.get("socket").cloned(),
        },
        "tcp" => RemoteKind::Tcp {
            address: props.get("address")?.clone(),
        },
        "ssh" => RemoteKind::Ssh {
            host: props.get("host")?.clone(),
        },
        "share" => RemoteKind::Share {
            passphrase: props.get("passphrase")?.clone(),
            hub: props.get("hub").cloned(),
        },
        _ => return None,
    };
    Some(RemoteConfig {
        name: name.to_string(),
        kind,
        autoconnect,
    })
}
