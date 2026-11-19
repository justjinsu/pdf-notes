//! Talking to an AI about the open PDF: summaries and questions.
//!
//! The person's own subscription is used through the official command-line
//! apps that log in with it — Claude Code (Claude Pro/Max) and Codex
//! (ChatGPT Plus/Pro). Ollama runs a free model on this computer. API keys work
//! too and are kept in the system's password store (macOS Keychain, Windows
//! Credential Manager), never in a file.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    /// Claude subscription, through Claude Code.
    ClaudeCode,
    /// ChatGPT subscription, through Codex.
    Codex,
    /// A model running on this computer with Ollama.
    Ollama,
    AnthropicKey,
    OpenAiKey,
}

impl Provider {
    pub const ALL: [Provider; 5] = [Provider::ClaudeCode, Provider::Codex, Provider::Ollama, Provider::AnthropicKey, Provider::OpenAiKey];

    pub fn name(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Claude · 내 구독",
            Provider::Codex => "ChatGPT · 내 구독",
            Provider::Ollama => "Ollama · 이 컴퓨터에서 (무료)",
            Provider::AnthropicKey => "Claude · API 키",
            Provider::OpenAiKey => "OpenAI · API 키",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Provider::ClaudeCode | Provider::AnthropicKey => "Claude",
            Provider::Codex | Provider::OpenAiKey => "ChatGPT",
            Provider::Ollama => "Ollama",
        }
    }

    pub fn how(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Claude Code 앱이 쓰는 Claude 구독 로그인을 그대로 써요. 터미널에서 `claude` 를 한 번 실행해 로그인해 두세요.",
            Provider::Codex => "Codex 앱이 쓰는 ChatGPT 구독 로그인을 그대로 써요. 터미널에서 `codex` 로 로그인해 두세요.",
            Provider::Ollama => "인터넷 없이 이 컴퓨터에서 돌아가요. Ollama 앱을 켜고 모델을 받아 두세요 (예: ollama pull llama3.1).",
            Provider::AnthropicKey => "console.anthropic.com 에서 만든 키. 쓴 만큼 따로 요금이 나와요.",
            Provider::OpenAiKey => "platform.openai.com 에서 만든 키. 쓴 만큼 따로 요금이 나와요.",
        }
    }

    fn keychain_service(self) -> &'static str {
        match self {
            Provider::OpenAiKey => "PDF Notes · OpenAI",
            _ => "PDF Notes · Anthropic",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: Option<Provider>,
    pub ollama_model: String,
    pub anthropic_model: String,
    pub openai_model: String,
    /// Answer language.
    pub language: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            provider: None,
            ollama_model: String::new(),
            anthropic_model: "claude-sonnet-5-5".into(),
            openai_model: "gpt-5".into(),
            language: "한국어".into(),
        }
    }
}

/// Apps started from Finder get a bare PATH, so look where installers put CLIs.
pub fn find_cli(name: &str) -> Option<PathBuf> {
    let home = crate::model::home_dir();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for d in [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin", "bin"] {
        dirs.push(home.join(d));
    }
    if cfg!(windows) {
        dirs.push(home.join("AppData/Roaming/npm"));
        dirs.push(home.join("AppData/Local/Programs/Ollama"));
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    // Windows programs end in .exe (or .cmd, when installed with npm).
    let names: Vec<String> = if cfg!(windows) { ["exe", "cmd", "bat"].iter().map(|e| format!("{name}.{e}")).collect() } else { vec![name.to_string()] };
    dirs.into_iter().flat_map(|d| names.iter().map(move |n| d.join(n)).collect::<Vec<_>>()).find(|p| p.is_file())
}

fn ollama_models() -> Result<Vec<String>, String> {
    let body = ureq::get("http://127.0.0.1:11434/api/tags").timeout(Duration::from_secs(2)).call().map_err(|_| "Ollama가 꺼져 있어요. Ollama 앱을 켜 주세요.".to_string())?.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(v["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()).unwrap_or_default())
}

/// Is this provider usable right now? (`Ok` = ready, with a short detail.)
pub fn check(p: Provider) -> Result<String, String> {
    match p {
        Provider::ClaudeCode => find_cli("claude").map(|_| "Claude Code 찾음".to_string()).ok_or_else(|| "Claude Code가 설치되어 있지 않아요".into()),
        Provider::Codex => find_cli("codex").map(|_| "Codex 찾음".to_string()).ok_or_else(|| "Codex가 설치되어 있지 않아요".into()),
        Provider::Ollama => {
            if find_cli("ollama").is_none() {
                return Err("Ollama가 설치되어 있지 않아요".into());
            }
            let m = ollama_models()?;
            if m.is_empty() { Err("받은 모델이 없어요 (ollama pull llama3.1)".into()) } else { Ok(format!("모델 {}개", m.len())) }
        }
        Provider::AnthropicKey | Provider::OpenAiKey => key_get(p).map(|_| "키 저장됨".to_string()).ok_or_else(|| "키를 넣어 주세요".into()),
    }
}

/// The first provider that works, in the order of `Provider::ALL`.
pub fn first_ready() -> Option<Provider> {
    Provider::ALL.into_iter().find(|p| *p != Provider::Ollama && check(*p).is_ok()).or_else(|| check(Provider::Ollama).ok().map(|_| Provider::Ollama))
}

/// What the button that stores a key says.
pub const KEY_SAVE_LABEL: &str = if cfg!(target_os = "macos") { "키체인에 저장" } else { "안전하게 저장" };

#[cfg(target_os = "macos")]
pub fn key_get(p: Provider) -> Option<String> {
    let out = Command::new("/usr/bin/security").args(["find-generic-password", "-a", "pdf-notes", "-s", p.keychain_service(), "-w"]).output().ok()?;
    let k = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !k.is_empty() { Some(k) } else { None }
}

#[cfg(target_os = "macos")]
pub fn key_set(p: Provider, key: &str) -> Result<(), String> {
    let st = Command::new("/usr/bin/security")
        .args(["add-generic-password", "-U", "-a", "pdf-notes", "-s", p.keychain_service(), "-w", key.trim()])
        .status()
        .map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err("키체인에 저장하지 못했어요".into()) }
}

/// Windows: the Credential Manager (control panel → "자격 증명 관리자").
#[cfg(windows)]
pub fn key_get(p: Provider) -> Option<String> {
    keyring::Entry::new(p.keychain_service(), "pdf-notes").ok()?.get_password().ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

#[cfg(windows)]
pub fn key_set(p: Provider, key: &str) -> Result<(), String> {
    keyring::Entry::new(p.keychain_service(), "pdf-notes").and_then(|e| e.set_password(key.trim())).map_err(|e| format!("키를 저장하지 못했어요: {e}"))
}

/// Elsewhere there is no password store we know: read the usual variables.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn key_get(p: Provider) -> Option<String> {
    std::env::var(if p == Provider::OpenAiKey { "OPENAI_API_KEY" } else { "ANTHROPIC_API_KEY" }).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn key_set(_p: Provider, _key: &str) -> Result<(), String> {
    Err("이 시스템에서는 ANTHROPIC_API_KEY / OPENAI_API_KEY 환경 변수로 키를 넣어 주세요.".into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub from_ai: bool,
    pub text: String,
}

fn scratch_dir() -> PathBuf {
    let d = crate::model::data_dir().join("ai");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// One whole conversation as plain text, for the command-line apps.
fn transcript(context: &str, history: &[Message]) -> String {
    let mut s = String::new();
    s.push_str("<document>\n");
    s.push_str(context);
    s.push_str("\n</document>\n\n");
    for m in history {
        s.push_str(if m.from_ai { "Assistant: " } else { "User: " });
        s.push_str(&m.text);
        s.push_str("\n\n");
    }
    s
}

fn run_cli(mut cmd: Command, input: &str, timeout: Duration) -> Result<String, String> {
    crate::model::no_console(&mut cmd);
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("실행하지 못했어요: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return Err("시간이 너무 오래 걸려서 멈췄어요.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() && !stdout.is_empty() {
        Ok(stdout)
    } else {
        Err(explain_error(&format!("{stdout}\n{stderr}")))
    }
}

fn explain_error(raw: &str) -> String {
    let l = raw.to_lowercase();
    if l.contains("newer version") || l.contains("upgrade") {
        "이 AI 앱을 최신 버전으로 업데이트해 주세요 (터미널: npm i -g @openai/codex 또는 claude update).".into()
    } else if l.contains("login") || l.contains("log in") || l.contains("not authenticated") || l.contains("unauthorized") || l.contains("401") {
        "로그인이 필요해요. 터미널에서 그 앱을 한 번 실행해 로그인해 주세요.".into()
    } else if l.contains("rate limit") || l.contains("usage limit") || l.contains("429") {
        "구독 사용량 한도에 닿았어요. 잠시 뒤에 다시 해 주세요.".into()
    } else {
        let line = raw.lines().map(str::trim).rev().find(|l| !l.is_empty()).unwrap_or("알 수 없는 오류");
        format!("AI가 답하지 못했어요: {}", line.chars().take(200).collect::<String>())
    }
}

pub fn system_prompt(language: &str) -> String {
    format!(
        "You are a research reading assistant inside a PDF reader. Answer in {language}. \
         Base answers on the document given between <document> tags (its text, the reader's highlights and memo). \
         Cite pages like (p. 3) when you use something from the text. If the document does not say, say so. \
         Be concise and well structured: short paragraphs or '-' bullet lists, no tables. Plain Markdown only."
    )
}

/// Ask `provider` the next reply to `history` about `context`.
pub fn ask(provider: Provider, s: &AiSettings, context: &str, history: &[Message]) -> Result<String, String> {
    let system = system_prompt(&s.language);
    match provider {
        Provider::ClaudeCode => {
            let exe = find_cli("claude").ok_or("Claude Code가 설치되어 있지 않아요")?;
            let mut c = Command::new(exe);
            c.current_dir(scratch_dir())
                .args(["-p", "--output-format", "text", "--no-session-persistence", "--strict-mcp-config", "--tools", "", "--system-prompt", &system]);
            run_cli(c, &(transcript(context, history) + "Assistant:"), Duration::from_secs(240))
        }
        Provider::Codex => {
            let exe = find_cli("codex").ok_or("Codex가 설치되어 있지 않아요")?;
            let out = scratch_dir().join(format!("codex-{}.txt", std::process::id()));
            let _ = std::fs::remove_file(&out);
            let mut c = Command::new(exe);
            c.current_dir(scratch_dir()).args(["exec", "--skip-git-repo-check", "--sandbox", "read-only", "--ephemeral", "--color", "never", "-o"]).arg(&out).arg("-");
            let prompt = format!("{system}\n\n{}Assistant:", transcript(context, history));
            let r = run_cli(c, &prompt, Duration::from_secs(300));
            match std::fs::read_to_string(&out) {
                Ok(t) if !t.trim().is_empty() => {
                    let _ = std::fs::remove_file(&out);
                    Ok(t.trim().to_string())
                }
                _ => r,
            }
        }
        Provider::Ollama => {
            let model = if s.ollama_model.trim().is_empty() { ollama_models()?.into_iter().next().ok_or("받은 모델이 없어요 (ollama pull llama3.1)")? } else { s.ollama_model.trim().to_string() };
            let mut msgs = vec![serde_json::json!({"role": "system", "content": format!("{system}\n\n<document>\n{context}\n</document>")})];
            for m in history {
                msgs.push(serde_json::json!({"role": if m.from_ai { "assistant" } else { "user" }, "content": m.text}));
            }
            let body = serde_json::json!({"model": model, "messages": msgs, "stream": false});
            let resp = ureq::post("http://127.0.0.1:11434/api/chat").timeout(Duration::from_secs(600)).send_json(body).map_err(|e| format!("Ollama: {e}"))?;
            let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
            v["message"]["content"].as_str().map(|s| s.trim().to_string()).ok_or_else(|| "Ollama가 빈 답을 보냈어요".into())
        }
        Provider::AnthropicKey => {
            let key = key_get(provider).ok_or("설정 → AI 에서 API 키를 넣어 주세요")?;
            let msgs: Vec<serde_json::Value> = history.iter().map(|m| serde_json::json!({"role": if m.from_ai { "assistant" } else { "user" }, "content": m.text})).collect();
            let body = serde_json::json!({
                "model": s.anthropic_model, "max_tokens": 2048,
                "system": format!("{system}\n\n<document>\n{context}\n</document>"),
                "messages": msgs,
            });
            let resp = ureq::post("https://api.anthropic.com/v1/messages")
                .set("x-api-key", &key)
                .set("anthropic-version", "2023-06-01")
                .timeout(Duration::from_secs(240))
                .send_json(body)
                .map_err(|e| explain_error(&e.to_string()))?;
            let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
            let text: String = v["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("")).unwrap_or_default();
            if text.is_empty() { Err("빈 답이 왔어요".into()) } else { Ok(text) }
        }
        Provider::OpenAiKey => {
            let key = key_get(provider).ok_or("설정 → AI 에서 API 키를 넣어 주세요")?;
            let mut msgs = vec![serde_json::json!({"role": "system", "content": format!("{system}\n\n<document>\n{context}\n</document>")})];
            for m in history {
                msgs.push(serde_json::json!({"role": if m.from_ai { "assistant" } else { "user" }, "content": m.text}));
            }
            let body = serde_json::json!({"model": s.openai_model, "messages": msgs});
            let resp = ureq::post("https://api.openai.com/v1/chat/completions")
                .set("Authorization", &format!("Bearer {key}"))
                .timeout(Duration::from_secs(240))
                .send_json(body)
                .map_err(|e| explain_error(&e.to_string()))?;
            let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
            v["choices"][0]["message"]["content"].as_str().map(|s| s.trim().to_string()).ok_or_else(|| "빈 답이 왔어요".into())
        }
    }
}

/// Ready-made requests.
pub const QUICK: [(&str, &str); 3] = [
    ("요약", "이 문서를 요약해 주세요: 한 줄 요지, 핵심 주장 3–5개, 방법, 결과, 한계. 각 항목에 쪽 표시를 붙여 주세요."),
    ("내 하이라이트 정리", "제가 표시한 하이라이트와 메모만 바탕으로, 이 문서에서 제가 중요하게 본 내용을 정리해 주세요. 빠진 중요한 부분이 있으면 마지막에 짧게 알려 주세요."),
    ("생각할 질문", "이 문서를 더 깊이 이해하기 위해 스스로 답해 볼 만한 질문 5개를 만들어 주세요. 각 질문에 관련 쪽을 붙여 주세요."),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_has_document_and_turns() {
        let t = transcript("TEXT", &[Message { from_ai: false, text: "요약해 줘".into() }]);
        assert!(t.contains("<document>\nTEXT\n</document>"));
        assert!(t.ends_with("User: 요약해 줘\n\n"));
        assert!(explain_error("The model requires a newer version of Codex").contains("업데이트"));
    }

    #[test]
    #[ignore]
    fn claude_code_answers() {
        let s = AiSettings::default();
        let r = ask(Provider::ClaudeCode, &s, "[p. 1] 사과는 빨갛다.", &[Message { from_ai: false, text: "사과는 무슨 색이에요? 한 문장으로.".into() }]).unwrap();
        println!("{r}");
        assert!(r.contains("빨"));
    }
}
