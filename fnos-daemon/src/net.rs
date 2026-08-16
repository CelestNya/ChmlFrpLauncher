//! 网络探测：ping 延迟 / 端口占用 / DNS 解析。
//!
//! 规格源：src-tauri/src/commands/ping.rs + ports.rs + process.rs resolve_domain_to_ip
//! （纯函数照抄，仅去掉 tauri 包装；`ping`/`netstat`/`lsof` 为系统命令，
//! 平台分支保持桌面版一致——Linux 为 daemon 主战场，Windows 供开发机调试）。

use serde::Serialize;
use std::collections::HashSet;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::collections::HashMap;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const PING_COUNT_FLAG: &str = "-n";
#[cfg(not(target_os = "windows"))]
const PING_COUNT_FLAG: &str = "-c";

#[derive(Serialize)]
pub struct PingResult {
    pub success: bool,
    pub latency: Option<f64>,
    pub error: Option<String>,
}

/// ping 一个主机（1 次探测，3s 超时 windows）。
pub async fn ping_host(host: String) -> Result<PingResult, String> {
    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new("ping");
        cmd.arg(PING_COUNT_FLAG).arg("1");

        #[cfg(target_os = "windows")]
        {
            cmd.arg("-w").arg("3000");
            cmd.creation_flags(0x08000000);
        }

        cmd.arg(&host);

        let output = cmd.output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if output.status.success() {
                    let latency_str = parse_ping_latency(&stdout);
                    let latency_bytes = if latency_str.is_none() {
                        parse_ping_latency_from_bytes(&output.stdout)
                    } else {
                        None
                    };

                    if let Some(latency) = latency_str.or(latency_bytes) {
                        Ok(PingResult {
                            success: true,
                            latency: Some(latency),
                            error: None,
                        })
                    } else {
                        let preview: Vec<String> =
                            stdout.lines().take(5).map(|s| s.to_string()).collect();
                        Ok(PingResult {
                            success: false,
                            latency: None,
                            error: Some(format!(
                                "Failed to parse ping output. First 5 lines: {:?}",
                                preview
                            )),
                        })
                    }
                } else {
                    let error_msg = if !stderr.is_empty() {
                        stderr.to_string()
                    } else {
                        stdout.to_string()
                    };
                    Ok(PingResult {
                        success: false,
                        latency: None,
                        error: Some(format!(
                            "Ping failed: {}",
                            error_msg.lines().next().unwrap_or("Unknown error")
                        )),
                    })
                }
            }
            Err(e) => Ok(PingResult {
                success: false,
                latency: None,
                error: Some(format!("Failed to execute ping: {}", e)),
            }),
        }
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

fn parse_ping_latency(output: &str) -> Option<f64> {
    #[cfg(target_os = "windows")]
    {
        for pattern in &["时间=", "time="] {
            if let Some(pos) = output.find(pattern) {
                let after_pattern = &output[pos + pattern.len()..];
                if let Some(ms_pos) = after_pattern.find("ms") {
                    let num_str = &after_pattern[..ms_pos];
                    let cleaned: String = num_str
                        .chars()
                        .filter(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    if !cleaned.is_empty() {
                        if let Ok(latency) = cleaned.parse::<f64>() {
                            return Some(latency);
                        }
                    }
                }
            }
        }

        for pattern in &["时间<", "time<"] {
            if let Some(pos) = output.find(pattern) {
                let after_pattern = &output[pos + pattern.len()..];
                if let Some(ms_pos) = after_pattern.find("ms") {
                    let num_str = &after_pattern[..ms_pos];
                    let cleaned: String = num_str
                        .chars()
                        .filter(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    if !cleaned.is_empty() {
                        if let Ok(latency) = cleaned.parse::<f64>() {
                            return Some(latency.max(0.1));
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(time_str) = output.split("time=").nth(1) {
            if let Some(ms_str) = time_str.split(" ms").next() {
                if let Ok(latency) = ms_str.trim().parse::<f64>() {
                    return Some(latency);
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn parse_ping_latency_from_bytes(bytes: &[u8]) -> Option<f64> {
    let ms_pattern = b"ms";
    let mut i = 0;

    while i < bytes.len().saturating_sub(ms_pattern.len()) {
        if bytes[i..i + ms_pattern.len()] == *ms_pattern {
            let end = i;
            let mut start = i;
            let mut found_digit = false;

            while start > 0 && (bytes[start - 1] == b' ' || bytes[start - 1] == b'\t') {
                start -= 1;
            }

            while start > 0 {
                let byte = bytes[start - 1];
                if byte.is_ascii_digit() || byte == b'.' {
                    found_digit = true;
                    start -= 1;
                } else if byte == b'=' || byte == b'<' {
                    if found_digit {
                        let num_bytes = &bytes[start..end];
                        if let Ok(num_str) = String::from_utf8(num_bytes.to_vec()) {
                            if let Ok(latency) = num_str.parse::<f64>() {
                                return Some(latency);
                            }
                        }
                    }
                    break;
                } else {
                    break;
                }
            }
        }
        i += 1;
    }

    None
}

#[cfg(not(target_os = "windows"))]
fn parse_ping_latency_from_bytes(_bytes: &[u8]) -> Option<f64> {
    None
}

// ---- 端口探测（照桌面版 ports.rs） ----

#[derive(Serialize)]
pub struct PortInfo {
    pub port: String,
    pub pid: String,
    pub process: String,
    pub protocol: String,
}

#[derive(Serialize)]
pub struct PortCheckResult {
    pub occupied: bool,
    pub pid: Option<String>,
    pub process: Option<String>,
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn deduplicate_ports(items: Vec<PortInfo>) -> Vec<PortInfo> {
    let mut seen = HashSet::new();
    let mut deduplicated = Vec::new();

    for item in items {
        let key = format!(
            "{}|{}|{}|{}",
            item.port, item.pid, item.process, item.protocol
        );
        if seen.insert(key) {
            deduplicated.push(item);
        }
    }

    deduplicated.sort_by(|a, b| {
        let a_port = a.port.parse::<u32>().unwrap_or(u32::MAX);
        let b_port = b.port.parse::<u32>().unwrap_or(u32::MAX);
        a_port
            .cmp(&b_port)
            .then_with(|| a.pid.cmp(&b.pid))
            .then_with(|| a.process.cmp(&b.process))
            .then_with(|| a.protocol.cmp(&b.protocol))
    });

    deduplicated
}

#[cfg(target_os = "windows")]
fn run_hidden_command(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args).creation_flags(CREATE_NO_WINDOW);
    let output = command.output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "windows")]
fn parse_tasklist_processes(tasklist_text: &str) -> HashMap<String, String> {
    tasklist_text
        .lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.trim().trim_matches('"').split("\",\"").collect();
            if columns.len() < 2 {
                return None;
            }
            Some((columns[1].to_string(), columns[0].to_string()))
        })
        .collect()
}

fn collect_ports() -> Vec<PortInfo> {
    #[cfg(target_os = "windows")]
    {
        let netstat_text = run_hidden_command("netstat", &["-ano"]).unwrap_or_default();
        let tasklist_text =
            run_hidden_command("tasklist", &["/FO", "CSV", "/NH"]).unwrap_or_default();
        let processes = parse_tasklist_processes(&tasklist_text);

        let mut result = Vec::new();

        for line in netstat_text.lines().skip(4) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let protocol = parts[0];
            let (address, pid) = match protocol {
                "TCP" if parts.len() >= 5 && parts[3] == "LISTENING" => (parts[1], parts[4]),
                "UDP" if parts.len() >= 4 => (parts[1], parts[3]),
                _ => continue,
            };

            if let Some(port) = address.split(':').last() {
                let process_name = processes.get(pid).cloned().unwrap_or_default();
                result.push(PortInfo {
                    port: port.to_string(),
                    pid: pid.to_string(),
                    process: process_name,
                    protocol: protocol.to_string(),
                });
            }
        }

        deduplicate_ports(result)
    }

    #[cfg(target_os = "linux")]
    {
        let output = Command::new("sh")
            .args(["-c", "netstat -lnptu 2>/dev/null | tail -n +3"])
            .output()
            .expect("failed to execute netstat");
        let text = String::from_utf8_lossy(&output.stdout);

        let mut result = Vec::new();
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 7 {
                let address = parts[3];
                let pid_proc = parts[6];
                if let Some(port) = address.split(':').last() {
                    let mut split = pid_proc.split('/');
                    let pid = split.next().unwrap_or("");
                    let process = split.next().unwrap_or("");
                    result.push(PortInfo {
                        port: port.to_string(),
                        pid: pid.to_string(),
                        process: process.to_string(),
                        protocol: parts[0].to_string(),
                    });
                }
            }
        }

        deduplicate_ports(result)
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("sh")
            .args(["-c", "lsof -n -P -iTCP -sTCP:LISTEN -iUDP"])
            .output()
            .expect("failed to execute lsof");
        let text = String::from_utf8_lossy(&output.stdout);

        let mut result = Vec::new();
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 9 {
                let process = parts[0];
                let pid = parts[1];
                let port_part = parts[8];
                if let Some(port) = port_part.split(':').last() {
                    result.push(PortInfo {
                        port: port.to_string(),
                        pid: pid.to_string(),
                        process: process.to_string(),
                        protocol: parts[7].to_string(),
                    });
                }
            }
        }

        deduplicate_ports(result)
    }
}

/// 获取本机监听端口列表。
pub async fn get_ports() -> Vec<PortInfo> {
    tokio::task::spawn_blocking(collect_ports)
        .await
        .unwrap_or_default()
}

/// 检查本地端口是否被占用。
pub async fn check_local_port(port: String) -> PortCheckResult {
    tokio::task::spawn_blocking(move || {
        let matched = collect_ports().into_iter().find(|item| item.port == port);

        match matched {
            Some(port_info) => PortCheckResult {
                occupied: true,
                pid: Some(port_info.pid),
                process: Some(port_info.process),
            },
            None => PortCheckResult {
                occupied: false,
                pid: None,
                process: None,
            },
        }
    })
    .await
    .unwrap_or(PortCheckResult {
        occupied: false,
        pid: None,
        process: None,
    })
}

/// 域名 → IP 解析（照桌面版 process.rs resolve_domain_to_ip）。
pub async fn resolve_domain_to_ip(domain: String) -> Result<Option<String>, String> {
    use std::net::ToSocketAddrs;

    tokio::task::spawn_blocking(move || {
        let addr_str = format!("{}:0", domain);
        Ok(addr_str
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .map(|addr| addr.ip().to_string()))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}