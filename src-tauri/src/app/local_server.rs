//! Start and stop the local llama-server used by the `local` provider.
//! Stop ends the process this app spawned, and also the process listening
//! on the provider port when the server was started outside the app.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::data::settings::{self, DEFAULT_LOCAL_MODEL_PATH, DEFAULT_LOCAL_SERVER_EXE};
use crate::error::{AppError, AppResult};

use super::state::AppState;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const HEALTH_POLLS: u32 = 120;
const HEALTH_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalServerStatus {
    pub healthy: bool,
    pub owned: bool,
}

#[tauri::command]
pub async fn local_llama_server_status(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
) -> Result<LocalServerStatus, AppError> {
    let provider = load_local_provider(&state, &provider_id)?;
    let (host, port) = endpoint_host_port(&provider.endpoint);
    let owned = reap_and_owned(&state.local_llama.child);
    let healthy = is_healthy(&host, port).await;
    Ok(LocalServerStatus { healthy, owned })
}

#[tauri::command]
pub async fn start_local_llama_server(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
) -> Result<LocalServerStatus, AppError> {
    let provider = load_local_provider(&state, &provider_id)?;
    let (host, port) = endpoint_host_port(&provider.endpoint);
    if reap_and_owned(&state.local_llama.child) && is_healthy(&host, port).await {
        return Ok(LocalServerStatus {
            healthy: true,
            owned: true,
        });
    }
    if is_healthy(&host, port).await {
        return Ok(LocalServerStatus {
            healthy: true,
            owned: false,
        });
    }

    {
        let mut starting = state
            .local_llama
            .starting
            .lock()
            .map_err(|_| AppError::Other("本地服务锁不可用".into()))?;
        if *starting {
            return Err(AppError::Other("本地服务正在启动".into()));
        }
        *starting = true;
    }
    let _starting = StartingGuard(&state.local_llama.starting);

    let exe = non_empty(&provider.local_server_exe, DEFAULT_LOCAL_SERVER_EXE);
    let model = non_empty(&provider.local_model_path, DEFAULT_LOCAL_MODEL_PATH);
    if !Path::new(&exe).is_file() {
        return Err(AppError::Other(format!("找不到 llama-server：{exe}")));
    }
    if !Path::new(&model).is_file() {
        return Err(AppError::Other(format!("找不到模型文件：{model}")));
    }

    let log_dir = Path::new(&model)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let stdout_path = log_dir.join("llama-server.log");
    let stderr_path = log_dir.join("llama-server.err.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)
        .map_err(|err| AppError::Other(format!("无法写入 {}：{err}", stdout_path.display())))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .map_err(|err| AppError::Other(format!("无法写入 {}：{err}", stderr_path.display())))?;

    let alias = provider
        .models
        .iter()
        .find(|model| model.capabilities.iter().any(|cap| cap == "rewrite"))
        .map(|model| model.id.clone())
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| "rewrite2-27b".to_string());

    let mut command = Command::new(&exe);
    if let Some(dir) = Path::new(&exe).parent() {
        command.current_dir(dir);
    }
    command
        .args([
            "-m",
            &model,
            "-ngl",
            "99",
            "-c",
            "4096",
            "--parallel",
            "1",
            "--repeat-penalty",
            "1.08",
            "--jinja",
            "--reasoning",
            "off",
            "--no-webui",
            "--host",
            &host,
            "--port",
            &port.to_string(),
            "-a",
            &alias,
            "--temp",
            "0.7",
            "--top-p",
            "0.9",
            "--top-k",
            "50",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|err| AppError::Other(format!("启动 llama-server 失败：{err}")))?;
    {
        let mut slot = state
            .local_llama
            .child
            .lock()
            .map_err(|_| AppError::Other("本地服务锁不可用".into()))?;
        if let Some(mut previous) = slot.replace(child) {
            let _ = previous.kill();
            let _ = previous.wait();
        }
    }

    for _ in 0..HEALTH_POLLS {
        if is_healthy(&host, port).await {
            return Ok(LocalServerStatus {
                healthy: true,
                owned: true,
            });
        }
        let exited = {
            let mut slot = state
                .local_llama
                .child
                .lock()
                .map_err(|_| AppError::Other("本地服务锁不可用".into()))?;
            match slot.as_mut().and_then(|child| child.try_wait().ok()) {
                Some(Some(status)) => {
                    slot.take();
                    Some(status.to_string())
                }
                Some(None) => None,
                None => Some("进程已结束".into()),
            }
        };
        if let Some(status) = exited {
            let tail = log_tail(&stderr_path);
            let detail = if tail.trim().is_empty() {
                status
            } else {
                format!("{status}\n{tail}")
            };
            return Err(AppError::Other(format!("llama-server 已退出：{detail}")));
        }
        tokio::time::sleep(HEALTH_WAIT).await;
    }

    if let Ok(mut slot) = state.local_llama.child.lock() {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    Err(AppError::Other(
        "本地服务启动超时，请查看模型目录下的 llama-server.err.log".into(),
    ))
}

#[tauri::command]
pub async fn stop_local_llama_server(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
) -> Result<LocalServerStatus, AppError> {
    let provider = load_local_provider(&state, &provider_id)?;
    let (host, port) = endpoint_host_port(&provider.endpoint);
    {
        let mut slot = state
            .local_llama
            .child
            .lock()
            .map_err(|_| AppError::Other("本地服务锁不可用".into()))?;
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    let pids = listener_pids(port)?;
    for pid in pids {
        kill_pid(pid)?;
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    let healthy = is_healthy(&host, port).await;
    if healthy {
        return Err(AppError::Other(
            "停止后服务仍在响应。请确认有权限结束占用该端口的进程。".into(),
        ));
    }
    Ok(LocalServerStatus {
        healthy: false,
        owned: false,
    })
}

/// PIDs in TCP LISTENING state on `port`. Remote `0.0.0.0:0` / `[::]:0` is
/// locale-independent, unlike the translated LISTENING label.
fn listener_pids(port: u16) -> AppResult<Vec<u32>> {
    let mut command = Command::new("netstat");
    command.args(["-ano", "-p", "tcp"]);
    hide_window(&mut command);
    let output = command
        .output()
        .map_err(|err| AppError::Other(format!("无法查询端口 {port}：{err}")))?;
    if !output.status.success() {
        return Err(AppError::Other(format!("无法查询端口 {port}")));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 || !fields[0].eq_ignore_ascii_case("tcp") {
            continue;
        }
        let remote = fields[2];
        let listening = remote == "0.0.0.0:0" || remote == "[::]:0" || remote == "*:*";
        if !listening || local_port(fields[1]) != Some(port) {
            continue;
        }
        let Ok(pid) = fields[fields.len() - 1].parse::<u32>() else {
            continue;
        };
        if pid > 0 && !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    Ok(pids)
}

fn local_port(addr: &str) -> Option<u16> {
    addr.rsplit_once(':')?.1.parse().ok()
}

fn kill_pid(pid: u32) -> AppResult<()> {
    if pid == 0 || pid == std::process::id() {
        return Ok(());
    }
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T", "/F"]);
    hide_window(&mut command);
    let output = command
        .output()
        .map_err(|err| AppError::Other(format!("无法结束进程 {pid}：{err}")))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    let gone = detail.contains("not found")
        || detail.contains("没有找到")
        || detail.contains("没有此任务");
    if gone {
        return Ok(());
    }
    Err(AppError::Other(format!(
        "无法结束进程 {pid}：{}",
        detail.trim()
    )))
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command;
}

struct StartingGuard<'a>(&'a Mutex<bool>);

impl Drop for StartingGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut starting) = self.0.lock() {
            *starting = false;
        }
    }
}

fn load_local_provider(state: &AppState, provider_id: &str) -> AppResult<settings::ModelProvider> {
    let conn = state.conn()?;
    let settings = settings::read(&conn)?;
    let provider = settings
        .model_services
        .into_iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| AppError::Other("找不到本地供应商".into()))?;
    if crate::ai::providers::normalize_sdk(&provider.sdk) != crate::ai::providers::LOCAL_SDK {
        return Err(AppError::Other(
            "只有本地供应商可以启动 llama-server".into(),
        ));
    }
    Ok(provider)
}

fn endpoint_host_port(endpoint: &str) -> (String, u16) {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return ("127.0.0.1".into(), 8080);
    };
    let host = url.host_str().unwrap_or("127.0.0.1").to_string();
    let port = url.port().unwrap_or(8080);
    (host, port)
}

fn non_empty(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn reap_and_owned(slot: &Mutex<Option<Child>>) -> bool {
    let Ok(mut slot) = slot.lock() else {
        return false;
    };
    let Some(child) = slot.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(None) => true,
        _ => {
            slot.take();
            false
        }
    }
}

async fn is_healthy(host: &str, port: u16) -> bool {
    let host_fmt = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let url = format!("http://{host_fmt}:{port}/health");
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
    else {
        return false;
    };
    matches!(client.get(url).send().await, Ok(resp) if resp.status().is_success())
}

fn log_tail(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 400 {
        text
    } else {
        chars[chars.len() - 400..].iter().collect()
    }
}
