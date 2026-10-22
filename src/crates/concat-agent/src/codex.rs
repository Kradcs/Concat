// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Official Codex CLI integration. Codex owns ChatGPT login and token refresh;
//! Concat never reads OAuth credentials or forwards them to an API endpoint.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Plan, instructions, parse_plan};

/// A separate Codex installation and credential store for Concat.
pub struct Codex {
    executable: PathBuf,
    home: PathBuf,
}

// These options apply to login and requests. Environment API keys are removed
// as well: choosing ChatGPT must never silently fall back to paid API usage.
const CONFIG: &[&str] = &[
    "forced_login_method=\"chatgpt\"",
    "model_provider=\"openai\"",
    "cli_auth_credentials_store=\"auto\"",
    "approval_policy=\"never\"",
    "sandbox_mode=\"read-only\"",
    "web_search=\"disabled\"",
    "features.shell_tool=false",
    "features.request_permissions_tool=false",
    "features.multi_agent=false",
    "features.multi_agent_v2=false",
    "features.apps=false",
    "features.connectors=false",
    "features.plugins=false",
    "features.hooks=false",
    "features.codex_hooks=false",
    "features.code_mode=false",
    "features.js_repl=false",
    "features.browser_use=false",
    "features.computer_use=false",
    "features.image_generation=false",
    "features.view_image=false",
    "features.memory_tool=false",
    "features.memories=false",
    "features.skip_host_skill_discovery=true",
];

impl Codex {
    /// Locate the native CLI shipped beside Concat; no Node.js is required.
    pub fn bundled() -> Result<Self, String> {
        let app =
            std::env::current_exe().map_err(|_| "Não foi possível localizar o aplicativo.")?;
        let executable = app
            .parent()
            .ok_or("Não foi possível localizar o aplicativo.")?
            .join("codex")
            .join(if cfg!(windows) { "codex.exe" } else { "codex" });
        let runtime = executable.with_file_name(if cfg!(windows) {
            "codex-code-mode-host.exe"
        } else {
            "codex-code-mode-host"
        });
        if !executable.is_file() || !runtime.is_file() {
            return Err("O componente Codex não foi encontrado. Instale a versão do Concat com login ChatGPT.".into());
        }
        let home = dirs::data_local_dir()
            .ok_or("Não foi possível localizar a pasta de dados do usuário.")?
            .join("Concat")
            .join("codex");
        let codex = Self { executable, home };
        codex.prepare()?;
        Ok(codex)
    }

    fn prepare(&self) -> Result<(), String> {
        fs::create_dir_all(&self.home).map_err(|_| "Não foi possível criar a pasta do Codex.")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.home, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Não foi possível proteger a pasta do Codex.")?;
        }
        Ok(())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .env("CODEX_HOME", &self.home)
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY")
            .env_remove("OPENAI_BASE_URL")
            .env_remove("CODEX_MANAGED_BY_NPM")
            .env_remove("CODEX_MANAGED_BY_BUN")
            .current_dir(&self.home);
        for setting in CONFIG {
            command.args(["-c", setting]);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // A GUI application should not flash a console on every request.
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        command
    }

    /// Check only Concat's own login; API-key authentication is not accepted.
    pub fn signed_in(&self, cancel: &AtomicBool) -> Result<bool, String> {
        let mut command = self.command();
        command.args(["login", "status"]);
        let result = run(command, None, Duration::from_secs(20), cancel)?;
        Ok(result.status.success() && result.stderr.contains("Logged in using ChatGPT"))
    }

    /// Open Codex's official browser login and wait for its localhost callback.
    pub fn login(&self, cancel: &AtomicBool) -> Result<(), String> {
        let mut command = self.command();
        command.arg("login");
        let result = run(command, None, Duration::from_secs(300), cancel)?;
        if !result.status.success() || !self.signed_in(cancel)? {
            return Err(
                "Não foi possível entrar. Tente novamente e conclua o login no navegador.".into(),
            );
        }
        Ok(())
    }

    /// Sign out of Concat without touching another Codex installation's login.
    pub fn logout(&self, cancel: &AtomicBool) -> Result<(), String> {
        let mut command = self.command();
        command.arg("logout");
        if !run(command, None, Duration::from_secs(30), cancel)?
            .status
            .success()
        {
            return Err("Não foi possível sair da conta. Tente novamente.".into());
        }
        Ok(())
    }

    /// Produce a validated editing proposal using the ChatGPT plan's Codex limits.
    pub fn propose(
        &self,
        prompt: &str,
        project: &Value,
        cancel: &AtomicBool,
    ) -> Result<Plan, String> {
        if prompt.trim().is_empty() || prompt.len() > 16_000 {
            return Err("Descreva uma edição em até 16000 bytes.".into());
        }
        let context = json!({"request":prompt,"project":project}).to_string();
        if context.len() > 2_000_000 {
            return Err("O projeto é grande demais para este pedido.".into());
        }
        if !self.signed_in(cancel)? {
            return Err("Entre com sua conta do ChatGPT antes de enviar o pedido.".into());
        }
        // An empty, private working directory prevents discovery of project
        // instructions. Requests are ephemeral and carry metadata via stdin,
        // never in process arguments or a project file.
        let work = tempfile::Builder::new()
            .prefix("request-")
            .tempdir_in(&self.home)
            .map_err(|_| "Não foi possível preparar o pedido.")?;
        let schema = work.path().join("response-schema.json");
        let output = work.path().join("response.json");
        // A string envelope avoids OpenAI strict-schema restrictions on the
        // engine's tagged unions. The inner array is still parsed as Command.
        fs::write(
            &schema,
            json!({
                "type":"object", "additionalProperties":false,
                "properties":{
                    "message":{"type":"string"},
                    "commandsJson":{"type":"string"}
                },
                "required":["message","commandsJson"]
            })
            .to_string(),
        )
        .map_err(|_| "Não foi possível preparar a resposta.")?;
        let input = format!(
            "{}\nFor this integration, encode the commands array as a JSON string in commandsJson \
             instead of commands. Return exactly message and commandsJson.\nProject and request:\n{}",
            instructions(),
            context
        );
        let mut stdin = tempfile::tempfile_in(work.path())
            .map_err(|_| "Não foi possível preparar o contexto.")?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|_| "Não foi possível escrever o contexto.")?;
        use std::io::{Seek, SeekFrom};
        stdin
            .seek(SeekFrom::Start(0))
            .map_err(|_| "Não foi possível ler o contexto.")?;
        let mut command = self.command();
        command
            .current_dir(work.path())
            .args([
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--strict-config",
                "--ephemeral",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "--json",
                "--output-schema",
            ])
            .arg(&schema)
            .arg("--output-last-message")
            .arg(&output)
            .arg("-");
        let result = run(command, Some(stdin), Duration::from_secs(240), cancel)?;
        if !result.status.success() {
            return Err("O Codex não concluiu o pedido. Verifique a conexão e os limites do seu plano; se necessário, entre novamente. Nenhuma edição foi aplicada.".into());
        }
        let mut reply = String::new();
        File::open(output)
            .map_err(|_| "O Codex não retornou uma proposta.")?
            .take(128_001)
            .read_to_string(&mut reply)
            .map_err(|_| "Resposta inválida do Codex.")?;
        if reply.len() > 128_000 {
            return Err("A resposta é grande demais. Peça uma edição menor.".into());
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Reply {
            message: String,
            commands_json: String,
        }
        let reply: Reply =
            serde_json::from_str(&reply).map_err(|_| "Resposta inválida do Codex.")?;
        let commands: Value = serde_json::from_str(&reply.commands_json)
            .map_err(|_| "O Codex retornou comandos inválidos.")?;
        parse_plan(&json!({"message":reply.message,"commands":commands}).to_string())
    }
}

#[derive(Debug)]
struct Output {
    status: ExitStatus,
    stderr: String,
}

// Own the child until it is reaped, including early I/O errors and cancellation.
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run(
    mut command: Command,
    stdin: Option<File>,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<Output, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Operação cancelada.".into());
    }
    let stderr = tempfile::tempfile().map_err(|_| "Não foi possível preparar o Codex.")?;
    let mut reader = stderr
        .try_clone()
        .map_err(|_| "Não foi possível preparar o Codex.")?;
    command
        .stdin(stdin.map(Stdio::from).unwrap_or_else(Stdio::null))
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    let mut child = Running(
        command
            .spawn()
            .map_err(|_| "Não foi possível iniciar o Codex.")?,
    );
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Operação cancelada.".into());
        }
        if started.elapsed() > timeout {
            return Err(
                "O Codex demorou demais. Tente novamente; nenhuma edição foi aplicada.".into(),
            );
        }
        if let Some(status) = child
            .0
            .try_wait()
            .map_err(|_| "Falha ao aguardar o Codex.")?
        {
            break status;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    use std::io::{Seek, SeekFrom};
    let _ = reader.seek(SeekFrom::Start(0));
    let mut bytes = Vec::new();
    let _ = reader.take(16_384).read_to_end(&mut bytes);
    Ok(Output {
        status,
        stderr: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

#[cfg(test)]
mod tests;
